use crate::commands::CatalogCommand;
use anyhow::{anyhow, bail, Context, Result};
use minisign::{KeyPair, PublicKey};
use modn_core::buckets::{catalog_statement, verify_catalog, CatalogPublicKey};
use serde_json::json;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Component, Path};

mod checkver;
mod checkver_output;
mod checkver_update;
mod checkver_upstream;
mod init;
mod keys;
mod maintenance;

use self::keys::{
    encoded_public, encoded_secret, fingerprint, load_secret_outside, read_password,
    read_public_key_text, require_directory, write_new_file, write_secret_key, write_signature,
};

const SECRET_KEY_COMMENT: &str = "modn catalog secret key";
const SIGNATURE_COMMENT: &str = "modn catalog signature";
const SECRET_KEY_SUFFIX: &str = ".key";
const PUBLIC_KEY_SUFFIX: &str = ".pub";
const PUBLIC_KEY_FILE: &str = "catalog.pub";
const SIGNATURE_FILE: &str = "catalog.sig";
const REVOCATION_KEY_FILE: &str = "revocation.pub";
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;

pub(super) fn run(command: &CatalogCommand, json_output: bool) -> Result<()> {
    match command {
        CatalogCommand::Keygen {
            out,
            name,
            password_env,
        } => keygen(out, name, password_env.as_deref(), json_output),
        CatalogCommand::Sign {
            dir,
            key,
            password_env,
            revocation_public_key,
        } => sign(
            dir,
            key,
            password_env.as_deref(),
            revocation_public_key.as_deref(),
            json_output,
        ),
        CatalogCommand::Verify { dir, key } => verify(dir, key, json_output),
        CatalogCommand::Hash { archive } => hash(archive, json_output),
        CatalogCommand::Init { dir, name, title } => init::run(dir, name, title, json_output),
        CatalogCommand::Checkver {
            dir,
            mod_id,
            update,
            keep_previous,
        } => checkver::run(dir, mod_id.as_deref(), json_output, *update, *keep_previous),
        CatalogCommand::Rotate {
            dir,
            key,
            new_public_key,
            password_env,
        } => maintenance::rotate(
            dir,
            key,
            new_public_key,
            password_env.as_deref(),
            json_output,
        ),
        CatalogCommand::Revoke {
            dir,
            revocation_key,
            revoked,
            replacement_public_key,
            password_env,
        } => maintenance::revoke(
            dir,
            revocation_key,
            revoked,
            replacement_public_key,
            password_env.as_deref(),
            json_output,
        ),
    }
}

fn keygen(out: &Path, name: &str, password_env: Option<&str>, json_output: bool) -> Result<()> {
    validate_key_name(name)?;
    let secret_path = out.join(format!("{name}{SECRET_KEY_SUFFIX}"));
    let public_path = out.join(format!("{name}{PUBLIC_KEY_SUFFIX}"));
    for path in [&secret_path, &public_path] {
        if fs::symlink_metadata(path).is_ok() {
            bail!(
                "signing key already exists: {}; remove it or choose another --name",
                path.display()
            );
        }
    }
    let password = match password_env {
        Some(variable) => Some(read_password(variable)?),
        None => None,
    };
    let pair = match password {
        Some(password) => KeyPair::generate_encrypted_keypair(Some(password)),
        None => KeyPair::generate_unencrypted_keypair(),
    }
    .context("cannot generate the signing key pair")?;
    let fingerprint = fingerprint(&pair.pk)?;
    let secret = encoded_secret(&pair)?;
    let public = encoded_public(&pair.pk)?;
    fs::create_dir_all(out)
        .with_context(|| format!("cannot create the key directory: {}", out.display()))?;
    write_secret_key(&secret_path, secret.as_bytes())?;
    if let Err(error) = write_new_file(&public_path, public.as_bytes()) {
        let _ = fs::remove_file(&secret_path);
        return Err(error);
    }
    if json_output {
        println!(
            "{}",
            json!({
                "fingerprint": fingerprint,
                "secret_key": secret_path,
                "public_key": public_path,
            })
        );
    } else {
        println!("Signing key fingerprint {fingerprint}");
        println!("Secret key: {}", secret_path.display());
        println!("Public key: {}", public_path.display());
        println!("Keep the secret key offline and never add it to a catalog.");
    }
    Ok(())
}

fn sign(
    root: &Path,
    key_path: &Path,
    password_env: Option<&str>,
    revocation_public_key: Option<&Path>,
    json_output: bool,
) -> Result<()> {
    require_directory(root, "catalog directory")?;
    let secret = load_secret_outside(root, key_path, password_env)?;
    let public = PublicKey::from_secret_key(&secret)
        .map_err(|_| anyhow!("secret key does not contain its public key"))?;
    let fingerprint = fingerprint(&public)?;
    let revocation = match revocation_public_key {
        Some(path) => {
            let text = read_public_key_text(path)?;
            let key = CatalogPublicKey::parse(&text)
                .with_context(|| format!("invalid revocation public key: {}", path.display()))?;
            Some((key, text))
        }
        None => None,
    };
    let (view, staging) = published_view(root, revocation.is_some())?;
    if let Some((_, text)) = &revocation {
        fs::write(view.join(REVOCATION_KEY_FILE), text).with_context(|| {
            format!(
                "cannot stage the revocation public key in {}",
                view.display()
            )
        })?;
    }
    let statement = catalog_statement(&view).context("cannot read the catalog")?;
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(statement.as_bytes()),
        Some(SIGNATURE_COMMENT),
        Some(SIGNATURE_COMMENT),
    )
    .map_err(|error| anyhow!("cannot sign the catalog statement: {error}"))?;
    drop(staging);
    let revocation_fingerprint = revocation.as_ref().map(|(key, _)| key.key_id().to_string());
    if let Some((_, text)) = &revocation {
        write_signature(&root.join(REVOCATION_KEY_FILE), text.as_bytes())?;
    }
    write_signature(
        &root.join(PUBLIC_KEY_FILE),
        encoded_public(&public)?.as_bytes(),
    )?;
    write_signature(
        &root.join(SIGNATURE_FILE),
        signature.into_string().as_bytes(),
    )?;
    if json_output {
        let mut value = json!({
            "fingerprint": fingerprint,
            "digest": statement.digest().as_str(),
            "signature": root.join(SIGNATURE_FILE),
            "public_key": root.join(PUBLIC_KEY_FILE),
        });
        if let Some(key_id) = revocation_fingerprint {
            value["revocation_key_id"] = json!(key_id);
            value["revocation_public_key"] = json!(root.join(REVOCATION_KEY_FILE));
        }
        println!("{value}");
    } else {
        println!("Signed catalog {} with key {fingerprint}", statement.name());
        if let Some(key_id) = revocation_fingerprint {
            println!("Pinned revocation key {key_id}");
        }
        println!("Digest: {}", statement.digest());
        println!(
            "Wrote {} and {}",
            root.join(SIGNATURE_FILE).display(),
            root.join(PUBLIC_KEY_FILE).display()
        );
    }
    Ok(())
}

fn verify(root: &Path, key_path: &Path, json_output: bool) -> Result<()> {
    require_directory(root, "catalog directory")?;
    let text = fs::read_to_string(key_path)
        .with_context(|| format!("cannot read public key: {}", key_path.display()))?;
    let key = CatalogPublicKey::parse(&text)
        .with_context(|| format!("invalid public key: {}", key_path.display()))?;
    let (view, staging) = published_view(root, false)?;
    let statement = catalog_statement(&view).context("cannot read the catalog")?;
    let signature = read_signature(&root.join(SIGNATURE_FILE))?;
    let verified = verify_catalog(&view, &key, &signature).map_err(|error| {
        anyhow!(
            "catalog {} does not verify with key {}: {error:#}. The current catalog digest is {}; a changed, added or deleted file changes it.",
            root.display(),
            key.key_id(),
            statement.digest()
        )
    })?;
    drop(staging);
    if json_output {
        println!(
            "{}",
            json!({
                "catalog": verified.statement.name(),
                "fingerprint": verified.key_id.to_string(),
                "digest": verified.statement.digest().as_str(),
            })
        );
    } else {
        println!(
            "Verified catalog {} with key {}",
            verified.statement.name(),
            verified.key_id
        );
        println!("Digest: {}", verified.statement.digest());
    }
    Ok(())
}

fn hash(archive: &Path, json_output: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(archive)
        .with_context(|| format!("cannot read archive: {}", archive.display()))?;
    if metadata.is_dir() {
        bail!(
            "archive must be a file, not a directory: {}",
            archive.display()
        );
    }
    if !metadata.is_file() {
        bail!("archive must be a regular file: {}", archive.display());
    }
    let mut file = fs::File::open(archive)
        .with_context(|| format!("cannot open archive: {}", archive.display()))?;
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut file, &mut hasher)
        .with_context(|| format!("cannot read archive: {}", archive.display()))?;
    let hex = hasher.finalize().to_hex().to_string();
    if json_output {
        println!("{}", json!({ "blake3": hex }));
    } else {
        println!("{hex}");
    }
    Ok(())
}

fn validate_key_name(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    let single =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if name.is_empty() || !single {
        bail!("key name must be a plain file name without a directory: {name}");
    }
    Ok(())
}

fn read_signature(path: &Path) -> Result<String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!(
                "catalog signature is missing: {}; run `modn catalog sign` first",
                path.display()
            )
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read catalog signature: {}", path.display()))
        }
    };
    if !metadata.is_file() {
        bail!(
            "catalog signature must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_SIGNATURE_BYTES {
        bail!("catalog signature exceeds size limit: {}", path.display());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read catalog signature: {}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| anyhow!("malformed catalog signature: not UTF-8"))
}

fn published_view(
    root: &Path,
    force_staging: bool,
) -> Result<(std::path::PathBuf, Option<tempfile::TempDir>)> {
    if !force_staging && fs::symlink_metadata(root.join(".git")).is_err() {
        return Ok((root.to_owned(), None));
    }
    let staging =
        tempfile::tempdir().context("cannot create a copy of the catalog without .git")?;
    copy_catalog_content(root, root, staging.path())?;
    Ok((staging.path().to_owned(), Some(staging)))
}

fn copy_catalog_content(root: &Path, source: &Path, target: &Path) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("cannot read catalog directory: {}", source.display()))?
    {
        let entry = entry
            .with_context(|| format!("cannot read catalog directory: {}", source.display()))?;
        let name = entry.file_name();
        if source == root && name == std::ffi::OsStr::new(".git") {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("cannot read catalog entry: {}", path.display()))?;
        if is_link_or_reparse(&metadata) {
            bail!("non-regular catalog entry: {}", path.display());
        }
        if metadata.is_dir() {
            let next = target.join(&name);
            fs::create_dir_all(&next)
                .with_context(|| format!("cannot copy catalog directory: {}", next.display()))?;
            copy_catalog_content(root, &path, &next)?;
        } else if metadata.is_file() {
            fs::copy(&path, target.join(&name))
                .with_context(|| format!("cannot copy catalog file: {}", path.display()))?;
        } else {
            bail!("non-regular catalog entry: {}", path.display());
        }
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
