//! Author tools: create a signing key pair, sign a catalog, verify one and pin archives.
//!
//! `keygen` and `sign` write the minisign formats that `modn bucket sync` verifies.
//! No command here reads or writes manager state, and none registers a catalog.

use crate::commands::CatalogCommand;
use anyhow::{anyhow, bail, Context, Result};
use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};
use modn_core::buckets::{catalog_statement, verify_catalog, CatalogPublicKey};
use serde_json::json;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Component, Path};

/// Untrusted comment of the secret key file that `keygen` writes.
const SECRET_KEY_COMMENT: &str = "modn catalog secret key";
/// Trusted and untrusted comment of a catalog signature.
const SIGNATURE_COMMENT: &str = "modn catalog signature";
const SECRET_KEY_SUFFIX: &str = ".key";
const PUBLIC_KEY_SUFFIX: &str = ".pub";
const PUBLIC_KEY_FILE: &str = "catalog.pub";
const SIGNATURE_FILE: &str = "catalog.sig";
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
        } => sign(dir, key, password_env.as_deref(), json_output),
        CatalogCommand::Verify { dir, key } => verify(dir, key, json_output),
        CatalogCommand::Hash { archive } => hash(archive, json_output),
    }
}

/// Write `<name>.key` and `<name>.pub` and report the fingerprint of the new key.
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

/// Sign the canonical statement of a catalog directory and publish its public key.
fn sign(root: &Path, key_path: &Path, password_env: Option<&str>, json_output: bool) -> Result<()> {
    require_directory(root, "catalog directory")?;
    require_regular_file(key_path, "secret key")?;
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("cannot resolve catalog directory: {}", root.display()))?;
    let canonical_key = fs::canonicalize(key_path)
        .with_context(|| format!("cannot resolve secret key: {}", key_path.display()))?;
    if canonical_key.starts_with(&canonical_root) {
        bail!(
            "secret key must live outside the catalog directory: {} is inside {}",
            key_path.display(),
            root.display()
        );
    }
    let text = fs::read_to_string(&canonical_key)
        .with_context(|| format!("cannot read secret key: {}", key_path.display()))?;
    let secret = decrypt_secret(&text, password_env)?;
    let public = PublicKey::from_secret_key(&secret)
        .map_err(|_| anyhow!("secret key does not contain its public key"))?;
    let fingerprint = fingerprint(&public)?;
    let (view, staging) = published_view(root)?;
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
    write_signature(
        &root.join(PUBLIC_KEY_FILE),
        encoded_public(&public)?.as_bytes(),
    )?;
    write_signature(
        &root.join(SIGNATURE_FILE),
        signature.into_string().as_bytes(),
    )?;
    if json_output {
        println!(
            "{}",
            json!({
                "fingerprint": fingerprint,
                "digest": statement.digest().as_str(),
                "signature": root.join(SIGNATURE_FILE),
                "public_key": root.join(PUBLIC_KEY_FILE),
            })
        );
    } else {
        println!("Signed catalog {} with key {fingerprint}", statement.name());
        println!("Digest: {}", statement.digest());
        println!(
            "Wrote {} and {}",
            root.join(SIGNATURE_FILE).display(),
            root.join(PUBLIC_KEY_FILE).display()
        );
    }
    Ok(())
}

/// Verify a catalog directory with the same minisign check that `modn bucket sync` uses.
fn verify(root: &Path, key_path: &Path, json_output: bool) -> Result<()> {
    require_directory(root, "catalog directory")?;
    let text = fs::read_to_string(key_path)
        .with_context(|| format!("cannot read public key: {}", key_path.display()))?;
    let key = CatalogPublicKey::parse(&text)
        .with_context(|| format!("invalid public key: {}", key_path.display()))?;
    let (view, staging) = published_view(root)?;
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

/// Print the BLAKE3 digest of an archive file, streamed, as its package `blake3` pin.
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

/// Refuse a key name that would escape the key directory or name another path.
fn validate_key_name(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    let single =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if name.is_empty() || !single {
        bail!("key name must be a plain file name without a directory: {name}");
    }
    Ok(())
}

/// Report the minisign key identifier as 16 uppercase hexadecimal digits.
fn fingerprint(public: &PublicKey) -> Result<String> {
    let bytes: [u8; 8] = public
        .keynum()
        .get(..8)
        .and_then(|keynum| keynum.try_into().ok())
        .ok_or_else(|| anyhow!("public key has no key identifier"))?;
    Ok(format!("{:016X}", u64::from_le_bytes(bytes)))
}

fn encoded_secret(pair: &KeyPair) -> Result<String> {
    Ok(pair
        .sk
        .to_box(Some(SECRET_KEY_COMMENT))
        .map_err(|error| anyhow!("cannot encode the secret key: {error}"))?
        .into_string())
}

fn encoded_public(public: &PublicKey) -> Result<String> {
    Ok(public
        .to_box()
        .map_err(|error| anyhow!("cannot encode the public key: {error}"))?
        .into_string())
}

/// Read a catalog signature file and refuse a missing, large or non-regular file.
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

/// A directory that holds the published content of a catalog.
///
/// A Git checkout has a root `.git` entry, which the catalog digest never
/// covers. A copy without it lets an author sign and verify the work tree.
fn published_view(root: &Path) -> Result<(std::path::PathBuf, Option<tempfile::TempDir>)> {
    if fs::symlink_metadata(root.join(".git")).is_err() {
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

/// Read the password that encrypts or decrypts a secret key from an environment variable.
fn read_password(variable: &str) -> Result<String> {
    match std::env::var(variable) {
        Ok(password) if !password.is_empty() => Ok(password),
        Ok(_) => bail!("environment variable {variable} holds an empty password"),
        Err(_) => bail!("environment variable {variable} is not set or is not UTF-8"),
    }
}

/// Decode a secret key file and decrypt it when it needs a password.
fn decrypt_secret(text: &str, password_env: Option<&str>) -> Result<SecretKey> {
    let box_text = secret_key_body(text)?;
    let boxed = SecretKeyBox::from_string(text)
        .map_err(|error| anyhow!("malformed secret key: {error}"))?;
    let raw = SecretKey::from_bytes(&decode_body(box_text)?)
        .map_err(|error| anyhow!("malformed secret key: {error}"))?;
    if !raw.is_encrypted() {
        return SecretKey::from_unencrypted_box(boxed)
            .map_err(|error| anyhow!("cannot read secret key: {error}"));
    }
    let variable = password_env.ok_or_else(|| {
        anyhow!("secret key is encrypted; pass --password-env <VAR> naming a variable that holds its password")
    })?;
    let password = read_password(variable)?;
    SecretKey::from_box(boxed, Some(password)).map_err(|error| {
        anyhow!("cannot decrypt secret key with the password in {variable}: {error}")
    })
}

/// The base64 line of a minisign secret key box, without its comment.
fn secret_key_body(text: &str) -> Result<&str> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines
        .next()
        .ok_or_else(|| anyhow!("malformed secret key: empty file"))?;
    if first.starts_with("untrusted comment:") {
        lines
            .next()
            .ok_or_else(|| anyhow!("malformed secret key: missing key line"))
    } else {
        Ok(first)
    }
}

fn decode_body(encoded: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("malformed secret key: invalid base64"))
}

fn require_directory(path: &Path, role: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("cannot read {role}: {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("{role} is not a directory: {}", path.display());
    }
    Ok(())
}

fn require_regular_file(path: &Path, role: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot read {role}: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{role} is not a regular file: {}", path.display());
    }
    Ok(())
}

/// Create a new owner-only file; an existing file is refused, never truncated.
fn write_secret_key(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot create secret key: {}", path.display()))?;
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error).with_context(|| format!("cannot write secret key: {}", path.display()));
    }
    restrict_secret_key(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_secret_key(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot restrict secret key: {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_secret_key(_path: &Path) -> Result<()> {
    // Windows inherits the access control list of the user's directory.
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot create public key: {}", path.display()))?;
    use std::io::Write as _;
    file.write_all(bytes)
        .with_context(|| format!("cannot write public key: {}", path.display()))?;
    Ok(())
}

/// Replace a catalog file atomically so a failure keeps the previous bytes.
fn write_signature(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("catalog file has no parent directory: {}", path.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .with_context(|| format!("cannot create a temporary file in {}", directory.display()))?;
    use std::io::Write as _;
    file.write_all(bytes)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.as_file()
        .sync_all()
        .with_context(|| format!("cannot flush {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o644))
            .with_context(|| format!("cannot set permissions on {}", path.display()))?;
    }
    file.persist(path)
        .map_err(|error| anyhow!("cannot replace {}: {}", path.display(), error.error))?;
    Ok(())
}
