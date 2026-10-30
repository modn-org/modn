//! Explicit metadata-only adoption of existing, unmodified game files.
//! No operation in this module creates, replaces or removes a game file.
use crate::planning::{insert_destination, prohibited_payload};
use crate::scan::DetectedMod;
use crate::state::Store;
use crate::transactions::{
    lock_store, validate_ownership_paths, validate_registered_adoption_roots,
    validate_transaction_root,
};
use crate::transactions::{ManagedPackage, OwnedFile};
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_GROUPS: usize = 2_048;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// An exact, read-only manifest proposed for explicit ownership transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct AdoptionCandidate {
    /// Game-relative selector identifying this package, not a filesystem write token.
    pub selector: String,
    /// Game policy's file classification.
    pub kind: String,
    /// Exact manually installed files proposed for ownership.
    pub files: Vec<OwnedFile>,
}

/// Read-only candidates. Preview never grants deletion permission.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct AdoptionPreview {
    /// Selected game identity.
    pub game_id: String,
    /// Canonical installation root.
    pub game_installation: PathBuf,
    /// Candidate groups that can be individually confirmed.
    pub packages: Vec<AdoptionCandidate>,
    /// Explicit warning about later deletion.
    pub consequence: &'static str,
}

const WARNING: &str =
    "CONFIRMATION GRANTS MODN PERMISSION TO DELETE THESE EXISTING MANUAL FILES ON LATER UNINSTALL.";

pub(crate) fn checked_file(root: &Path, relative: &str) -> Result<(PathBuf, u64, String)> {
    if relative.len() > 256
        || !relative.contains('/')
        || !crate::archive::validated_name(relative).is_ok_and(|name| name == relative)
        || prohibited_payload(relative)
    {
        bail!("invalid adoption file path: {relative}");
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = fs::symlink_metadata(&path)
            .with_context(|| format!("cannot inspect adoption path {}", path.display()))?;
        if meta.file_type().is_symlink()
            || (index + 1 == parts.len() && !meta.file_type().is_file())
            || (index + 1 < parts.len() && !meta.file_type().is_dir())
        {
            bail!(
                "linked, special or non-file adoption path: {}",
                path.display()
            );
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(root) {
            bail!("adoption path escapes installation root");
        }
        // Reject short-name aliases and case/Unicode alternate spellings.
        let exact = path
            .parent()
            .context("missing adoption parent")?
            .read_dir()?
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name() == std::ffi::OsStr::new(part));
        if !exact {
            bail!("aliased adoption path: {}", path.display());
        }
    }
    let before = fs::metadata(&path)?;
    if before.len() > MAX_FILE_BYTES {
        bail!("adoption file exceeds 4 GiB");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            bail!("hard-linked files cannot be adopted");
        }
    }
    #[cfg(windows)]
    {
        let system_root =
            std::env::var_os("SystemRoot").context("cannot verify Windows hard-link count")?;
        let fsutil = PathBuf::from(system_root).join("System32/fsutil.exe");
        let output = std::process::Command::new(fsutil)
            .args(["hardlink", "list"])
            .arg(&path)
            .output()?;
        if !output.status.success() {
            bail!(
                "cannot verify Windows hard-link count for {}",
                path.display()
            );
        }
        let listing = String::from_utf8(output.stdout).context("invalid hard-link listing")?;
        if listing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        {
            bail!("hard-linked files cannot be adopted");
        }
    }
    let identity = same_file::Handle::from_path(&path)?;
    let mut file = File::open(&path)?;
    let mut hash = Sha256::new();
    let bytes = io::copy(&mut (&mut file).take(before.len() + 1), &mut hash)?;
    if bytes != before.len()
        || fs::metadata(&path)?.len() != before.len()
        || same_file::Handle::from_path(&path)? != identity
    {
        bail!("adoption file changed while reading: {}", path.display());
    }
    Ok((path, bytes, format!("{:x}", hash.finalize())))
}

fn validate_groups(
    root: &Path,
    detected: &[DetectedMod],
    groups: Vec<crate::game_modules::policy::AdoptionGroup>,
) -> Result<Vec<AdoptionCandidate>> {
    if groups.len() > MAX_GROUPS {
        bail!("too many adoption groups");
    }
    let recognized: HashMap<String, String> = detected
        .iter()
        .map(|item| {
            (
                item.relative_path.to_string_lossy().replace('\\', "/"),
                item.kind.clone(),
            )
        })
        .collect();
    let mut used = HashSet::new();
    let mut selectors = BTreeSet::new();
    let mut packages = Vec::new();
    for group in groups {
        if group.selector.len() > 256
            || group.files.is_empty()
            || group.files.len() > MAX_GROUPS
            || !crate::archive::validated_name(&group.selector)
                .is_ok_and(|name| name == group.selector)
            || insert_destination(&mut selectors, &group.selector).is_some()
        {
            bail!("invalid or duplicate adoption group");
        }
        if group.kind.is_empty()
            || group.kind.len() > 64
            || !group
                .kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bail!("invalid adoption kind");
        }
        let mut files = Vec::new();
        let prefix = format!("{}/", group.selector);
        let required = recognized
            .keys()
            .filter(|path| path.starts_with(&prefix))
            .count();
        if required > 0 && group.files.len() != required {
            bail!("incomplete adoption group: {}", group.selector);
        }
        let mut targets = BTreeSet::new();
        let mut total = 0;
        for relative in group.files {
            if recognized.get(&relative) != Some(&group.kind) || !used.insert(relative.clone()) {
                bail!("ambiguous or unknown adoption file: {relative}");
            }
            if relative != group.selector && !relative.starts_with(&format!("{}/", group.selector))
            {
                bail!("adoption group includes file outside selector: {relative}");
            }
            if let Some(code) = insert_destination(&mut targets, &relative) {
                bail!("{code}: {relative}");
            }
            let (_, bytes, sha256) = checked_file(root, &relative)?;
            total += bytes;
            if total > MAX_BYTES {
                bail!("adoption group exceeds 8 GiB");
            }
            files.push(OwnedFile {
                destination: relative,
                sha256,
                bytes,
            });
        }
        files.sort_by(|a, b| a.destination.cmp(&b.destination));
        packages.push(AdoptionCandidate {
            selector: group.selector,
            kind: group.kind,
            files,
        });
    }
    packages.sort_by(|a, b| a.selector.cmp(&b.selector));
    Ok(packages)
}

/// Enumerate exact, game-policy-selected packages without initializing manager state.
pub fn preview_adoption_for(
    store: Option<&Store>,
    game_id: &str,
    game: &Path,
) -> Result<AdoptionPreview> {
    let root = crate::discovery::validate_installation_for(game_id, game)?;
    let report = crate::game_modules::policy::scan(game_id, &root, 20_000)?;
    if report.incomplete {
        bail!("adoption refused: scan is incomplete or contains unreadable entries");
    }
    let groups = crate::game_modules::policy::adoption_groups(game_id, &root, &report.mods)?;
    let mut packages = validate_groups(&root, &report.mods, groups)?;
    if let Some(store) = store {
        let installed = store.installed(&root)?;
        let owned: HashSet<_> = installed
            .into_iter()
            .flat_map(|p| p.files)
            .map(|file| file.destination.to_lowercase())
            .collect();
        packages.retain(|p| {
            !p.files
                .iter()
                .any(|file| owned.contains(&file.destination.to_lowercase()))
        });
    }
    Ok(AdoptionPreview {
        game_id: game_id.into(),
        game_installation: root,
        packages,
        consequence: WARNING,
    })
}

impl Store {
    /// Transfer ownership of ONE explicitly approved, exact candidate; no game I/O writes.
    pub fn adopt(
        &mut self,
        game_id: &str,
        game: &Path,
        approved: &AdoptionCandidate,
    ) -> Result<ManagedPackage> {
        self.adopt_inner(game_id, game, approved, None)
    }

    fn adopt_inner(
        &mut self,
        game_id: &str,
        game: &Path,
        approved: &AdoptionCandidate,
        fail_after: Option<usize>,
    ) -> Result<ManagedPackage> {
        let _lock = lock_store(self)?;
        self.ensure_no_pending()?;
        let game = validate_transaction_root(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from game directory");
        }
        // Preview is not an approval token: re-run game policy and verify ALL
        // selected contents, sizes, hashes, links and names under the store lock.
        let current = preview_adoption_for(Some(self), game_id, &game)?;
        let selected = current
            .packages
            .into_iter()
            .find(|p| p.selector == approved.selector)
            .context("adoption selection is no longer available")?;
        if selected != *approved {
            bail!("adoption proposal changed since preview");
        }
        validate_registered_adoption_roots(self, game_id, &game)?;
        validate_ownership_paths(
            self,
            &game,
            selected.files.iter().map(|file| file.destination.as_str()),
        )?;
        let mut hasher = Sha256::new();
        hasher.update(game.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(selected.selector.as_bytes());
        for file in &selected.files {
            hasher.update([0]);
            hasher.update(file.destination.as_bytes());
            hasher.update(file.sha256.as_bytes());
        }
        let package_id = format!("adopt-{:x}", hasher.finalize());
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256,origin)
             VALUES (?1,?2,NULL,'manual')",
            params![package_id, game.to_string_lossy().as_ref()],
        )?;
        for (index, file) in selected.files.iter().enumerate() {
            tx.execute(
                "INSERT INTO managed_files(game_path,path_key,relative_path,package_id,sha256,bytes)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![game.to_string_lossy().as_ref(), file.destination.to_lowercase(),
                    file.destination, package_id, file.sha256, file.bytes as i64],
            )?;
            if fail_after == Some(index + 1) {
                bail!("injected failure during adoption transaction");
            }
        }
        tx.commit()?;
        if fail_after == Some(0) {
            bail!("injected failure after adoption commit");
        }
        Ok(ManagedPackage {
            id: package_id,
            game_installation: game,
            archive_sha256: None,
            origin: "manual".into(),
            files: selected.files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Store, PathBuf) {
        let temp = tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).unwrap();
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
        let folder = game.join("red4ext/plugins/Custom");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("one.dll"), b"one").unwrap();
        fs::write(folder.join("two.bin"), b"two").unwrap();
        let store = Store::open(&temp.path().join("state/state.sqlite")).unwrap();
        (temp, store, game)
    }

    #[test]
    fn failed_sqlite_transaction_rolls_back_all_ownership_without_touching_game() {
        let (_temp, mut store, game) = fixture();
        let preview = preview_adoption_for(Some(&store), "cyberpunk-2077", &game).unwrap();
        let selection = &preview.packages[0];
        assert_eq!(selection.files.len(), 2);
        assert!(store
            .adopt_inner("cyberpunk-2077", &game, selection, Some(1))
            .is_err());
        assert!(store.installed(&game).unwrap().is_empty());
        assert_eq!(store.recover().unwrap(), 0);
        assert_eq!(
            fs::read(game.join("red4ext/plugins/Custom/one.dll")).unwrap(),
            b"one"
        );
        assert_eq!(
            fs::read(game.join("red4ext/plugins/Custom/two.bin")).unwrap(),
            b"two"
        );
        let before = preview_adoption_for(Some(&store), "cyberpunk-2077", &game).unwrap();
        assert_eq!(before.packages.len(), 1);
        assert!(store
            .adopt_inner("cyberpunk-2077", &game, selection, Some(0))
            .is_err());
        assert_eq!(store.installed(&game).unwrap().len(), 1);
        assert!(store.adopt("cyberpunk-2077", &game, selection).is_err());
        assert_eq!(store.recover().unwrap(), 0);
    }

    #[test]
    fn selected_file_changing_after_preview_prevents_adoption() {
        let (_temp, mut store, game) = fixture();
        let preview = preview_adoption_for(Some(&store), "cyberpunk-2077", &game).unwrap();
        fs::write(game.join("red4ext/plugins/Custom/two.bin"), b"changed").unwrap();
        assert!(store
            .adopt("cyberpunk-2077", &game, &preview.packages[0])
            .is_err());
        assert!(store.installed(&game).unwrap().is_empty());
    }

    #[test]
    fn malicious_luau_groups_cannot_omit_or_overlap_scanned_files() {
        use crate::game_modules::policy::AdoptionGroup;
        let (_temp, _store, game) = fixture();
        let game = game.canonicalize().unwrap();
        let detected = vec![
            DetectedMod {
                kind: "red4ext-plugin".into(),
                relative_path: PathBuf::from("red4ext/plugins/Custom/one.dll"),
                managed: false,
                package_id: None,
                integrity: None,
            },
            DetectedMod {
                kind: "red4ext-plugin".into(),
                relative_path: PathBuf::from("red4ext/plugins/Custom/two.bin"),
                managed: false,
                package_id: None,
                integrity: None,
            },
        ];
        let group = AdoptionGroup {
            selector: "red4ext/plugins/Custom".into(),
            kind: "red4ext-plugin".into(),
            files: vec!["red4ext/plugins/Custom/one.dll".into()],
        };
        assert!(validate_groups(&game, &detected, vec![group]).is_err());
        let group = AdoptionGroup {
            selector: "red4ext/plugins/Custom".into(),
            kind: "red4ext-plugin".into(),
            files: vec![
                "red4ext/plugins/Custom/one.dll".into(),
                "red4ext/plugins/Custom/two.bin".into(),
            ],
        };
        let valid = validate_groups(&game, &detected, vec![group]).unwrap();
        assert_eq!(valid[0].files.len(), 2);
        let forged = AdoptionGroup {
            selector: "red4ext/plugins/Custom".into(),
            kind: "legacy-archive".into(),
            files: vec![
                "red4ext/plugins/Custom/one.dll".into(),
                "red4ext/plugins/Custom/two.bin".into(),
            ],
        };
        assert!(validate_groups(&game, &detected, vec![forged]).is_err());
    }

    #[test]
    fn hardlinks_cannot_be_adopted() {
        let (temp, _store, game) = fixture();
        fs::hard_link(
            game.join("red4ext/plugins/Custom/one.dll"),
            temp.path().join("external-hardlink"),
        )
        .unwrap();
        assert!(preview_adoption_for(None, "cyberpunk-2077", &game).is_err());
    }

    #[test]
    fn existing_pr14_ledger_migrates_without_changing_archive_provenance() {
        let temp = tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO managed_packages (id,game_path,archive_sha256)
            VALUES ('old','/fixture','abcd');",
        )
        .unwrap();
        drop(conn);
        let store = Store::open(&db).unwrap();
        let (digest, origin): (Option<String>, String) = store
            .connection
            .query_row(
                "SELECT archive_sha256,origin FROM managed_packages WHERE id='old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(digest.as_deref(), Some("abcd"));
        assert_eq!(origin, "archive");
        let nullable: i64 = store.connection.query_row(
            "SELECT \"notnull\" FROM pragma_table_info('managed_packages') WHERE name='archive_sha256'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(nullable, 0);
        drop(store);
        let reopened = Store::open(&db).unwrap();
        assert_eq!(
            reopened
                .connection
                .query_row("SELECT COUNT(*) FROM managed_packages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
