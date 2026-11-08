use super::lock::{acquire_lock, LOCK_WAIT};
use super::signature::{main_bucket_public_key, verify_catalog, CatalogKeyId};
use super::BucketStore;
use crate::acquisition::MAX_DOWNLOAD_BYTES;
use crate::{archive, bucket_digest};
use anyhow::{bail, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;

fn directory_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => bail!("unsafe main bucket update directory: {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_directory(path: &Path) -> Result<()> {
    if directory_exists(path)? {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn recover(root: &Path) -> Result<()> {
    let main = root.join("main");
    let aside = root.join(".main-aside");
    let rollback = root.join(".main-rollback");
    if directory_exists(&aside)? {
        if directory_exists(&main)? {
            remove_directory(&aside)?;
        } else {
            let previous = if directory_exists(&rollback)? {
                &rollback
            } else {
                &aside
            };
            fs::rename(previous, &main).context("restore interrupted main bucket update")?;
            remove_directory(&aside)?;
        }
    }
    remove_directory(&rollback)?;
    remove_directory(&root.join(".main-staging"))
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &destination)
                .context("copy previous main bucket for rollback")?;
            OpenOptions::new()
                .write(true)
                .open(destination)?
                .sync_all()
                .context("sync previous main bucket rollback file")?;
        } else {
            bail!("non-regular previous main bucket entry");
        }
    }
    Ok(())
}

fn swap(root: &Path, had_main: bool, delete_old: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    let main = root.join("main");
    let stage = root.join(".main-staging");
    let aside = root.join(".main-aside");
    let rollback = root.join(".main-rollback");
    if had_main {
        copy_tree(&main, &rollback)?;
        fs::rename(&main, &aside).context("move previous main bucket aside")?;
    }
    if let Err(error) = fs::rename(&stage, &main) {
        if had_main {
            fs::rename(&aside, &main).context("restore previous main bucket")?;
        }
        return Err(error.into());
    }
    if let Err(error) = delete_old(&aside) {
        fs::rename(&main, &stage).context("undo main bucket swap")?;
        if had_main {
            fs::rename(&rollback, &main).context("restore previous main bucket")?;
        }
        return Err(error);
    }
    let _ = remove_directory(&rollback);
    Ok(())
}

fn read_signature(mut reader: impl Read) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_SIGNATURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SIGNATURE_BYTES {
        bail!("main bucket signature exceeds size limit");
    }
    String::from_utf8(bytes).context("malformed catalog signature: not UTF-8")
}

fn extract(path: &Path, stage: &Path) -> Result<()> {
    let inspection = archive::inspect(path)?;
    fs::create_dir(stage)?;
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    for file in inspection.files {
        let target = stage.join(&file.entry);
        fs::create_dir_all(target.parent().context("missing bucket entry parent")?)?;
        let mut input = zip.by_name(file.original_entry.as_deref().unwrap_or(&file.entry))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        let copied = io::copy(&mut input.by_ref().take(file.bytes + 1), &mut output)?;
        if copied != file.bytes {
            bail!("main bucket extraction size mismatch: {}", file.entry);
        }
        output.sync_all()?;
    }
    Ok(())
}

impl BucketStore {
    /// Verify and install a signed main bucket ZIP. Recover an interrupted swap before updating.
    /// Return the installed tree digest, whether its contents changed, and the signer key.
    pub fn update_main<R: Read, S: Read>(
        &self,
        embedded_key: &str,
        source: impl FnOnce() -> Result<R>,
        signature: impl FnOnce() -> Result<S>,
    ) -> Result<(String, bool, CatalogKeyId)> {
        let key = main_bucket_public_key(embedded_key)?;
        self.check_registry_root()?;
        fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join(".main-update.lock");
        let _lock = acquire_lock(
            &lock_path,
            "unsafe main bucket update lock",
            "main bucket update is already running",
            LOCK_WAIT,
        )?;
        recover(&self.root)?;
        let signature_text = read_signature(signature()?)?;
        let mut source = source()?;
        let mut zip = tempfile::Builder::new()
            .suffix(".zip")
            .tempfile_in(&self.root)?;
        let copied = io::copy(&mut source.by_ref().take(MAX_DOWNLOAD_BYTES + 1), &mut zip)?;
        if copied > MAX_DOWNLOAD_BYTES {
            bail!("main bucket download exceeds size limit");
        }
        zip.as_file().sync_all()?;
        let stage = self.root.join(".main-staging");
        let main = self.root.join("main");
        let result = (|| {
            extract(zip.path(), &stage)?;
            let verified = verify_catalog(&stage, &key, &signature_text)?;
            if verified.statement.name() != "main" {
                bail!(
                    "main bucket release names its catalog \"{}\"; expected \"main\"",
                    verified.statement.name()
                );
            }
            self.validate_main(&stage)?;
            let digest = bucket_digest::directory_digest(&stage)?;
            let had_main = directory_exists(&main)?;
            let changed = !had_main || bucket_digest::directory_digest(&main)? != digest;
            swap(&self.root, had_main, remove_directory)?;
            Ok((digest, changed, verified.key_id))
        })();
        let cleanup = remove_directory(&stage);
        match result {
            Ok(value) => {
                cleanup?;
                Ok(value)
            }
            Err(error) => {
                cleanup.context("clean failed main bucket update")?;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{recover, swap};
    use crate::buckets::BucketStore;
    use fs2::FileExt;
    use minisign::KeyPair;
    use std::fs::{self, OpenOptions};
    use std::io;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn partial_old_copy_deletion_restores_complete_previous_tree() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("main")).unwrap();
        fs::write(root.path().join("main/first"), b"first").unwrap();
        fs::write(root.path().join("main/second"), b"second").unwrap();
        fs::create_dir(root.path().join(".main-staging")).unwrap();
        fs::write(root.path().join(".main-staging/new"), b"new").unwrap();
        let result = swap(root.path(), true, |aside| {
            fs::remove_file(aside.join("first"))?;
            anyhow::bail!("injected partial cleanup failure");
        });
        assert!(result.is_err());
        assert_eq!(fs::read(root.path().join("main/first")).unwrap(), b"first");
        assert_eq!(
            fs::read(root.path().join("main/second")).unwrap(),
            b"second"
        );
        recover(root.path()).unwrap();
        assert!(!root.path().join(".main-aside").exists());
        assert!(!root.path().join(".main-staging").exists());
    }

    #[test]
    fn failed_stage_rename_restores_previous_copy() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("main")).unwrap();
        fs::write(root.path().join("main/old"), b"old").unwrap();
        assert!(swap(root.path(), true, super::remove_directory).is_err());
        assert_eq!(fs::read(root.path().join("main/old")).unwrap(), b"old");
        recover(root.path()).unwrap();
        assert!(!root.path().join(".main-rollback").exists());
    }

    #[test]
    fn recovery_uses_complete_rollback_after_interrupted_cleanup() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".main-aside")).unwrap();
        fs::write(root.path().join(".main-aside/remaining"), b"partial").unwrap();
        fs::create_dir(root.path().join(".main-rollback")).unwrap();
        fs::write(root.path().join(".main-rollback/old"), b"complete").unwrap();
        fs::create_dir(root.path().join(".main-staging")).unwrap();
        recover(root.path()).unwrap();
        assert_eq!(fs::read(root.path().join("main/old")).unwrap(), b"complete");
        assert!(!root.path().join(".main-aside").exists());
        assert!(!root.path().join(".main-rollback").exists());
        assert!(!root.path().join(".main-staging").exists());
    }

    fn held_update_lock(state: &std::path::Path) -> (std::fs::File, BucketStore, String) {
        let store = BucketStore::new(state);
        fs::create_dir_all(state.join("buckets")).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(state.join("buckets/.main-update.lock"))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();
        let key = KeyPair::generate_unencrypted_keypair().unwrap();
        let text = key.pk.to_box().unwrap().into_string();
        (lock, store, text)
    }

    #[test]
    fn update_waits_for_a_short_held_lock() {
        let state = tempfile::tempdir().unwrap();
        let (lock, store, text) = held_update_lock(state.path());

        thread::scope(|scope| {
            let update = scope.spawn(|| {
                store.update_main(
                    &text,
                    || Ok(io::empty()),
                    || -> anyhow::Result<io::Empty> { anyhow::bail!("injected signature failure") },
                )
            });
            thread::sleep(Duration::from_millis(200));
            drop(lock);
            let error = update.join().unwrap().unwrap_err().to_string();
            assert_eq!(error, "injected signature failure");
        });
    }

    #[test]
    fn update_reports_already_running_while_the_lock_stays_held() {
        let state = tempfile::tempdir().unwrap();
        let (lock, store, text) = held_update_lock(state.path());

        let error = store
            .update_main(
                &text,
                || Ok(io::empty()),
                || -> anyhow::Result<io::Empty> { anyhow::bail!("unused signature") },
            )
            .unwrap_err()
            .to_string();
        assert_eq!(error, "main bucket update is already running");
        drop(lock);
    }
}
