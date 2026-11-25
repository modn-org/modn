use anyhow::{anyhow, bail, Context, Result};
use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};
use std::fs;
use std::path::Path;

const MAX_PUBLIC_KEY_BYTES: u64 = 4 * 1024;

pub(super) fn fingerprint(public: &PublicKey) -> Result<String> {
    let bytes: [u8; 8] = public
        .keynum()
        .get(..8)
        .and_then(|keynum| keynum.try_into().ok())
        .ok_or_else(|| anyhow!("public key has no key identifier"))?;
    Ok(format!("{:016X}", u64::from_le_bytes(bytes)))
}

pub(super) fn encoded_secret(pair: &KeyPair) -> Result<String> {
    Ok(pair
        .sk
        .to_box(Some(super::SECRET_KEY_COMMENT))
        .map_err(|error| anyhow!("cannot encode the secret key: {error}"))?
        .into_string())
}

pub(super) fn encoded_public(public: &PublicKey) -> Result<String> {
    Ok(public
        .to_box()
        .map_err(|error| anyhow!("cannot encode the public key: {error}"))?
        .into_string())
}

pub(super) fn load_secret_outside(
    root: &Path,
    key_path: &Path,
    password_env: Option<&str>,
) -> Result<SecretKey> {
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
    decrypt_secret(&text, password_env)
}

pub(super) fn read_public_key_text(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot read public key: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("public key is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_PUBLIC_KEY_BYTES {
        bail!("public key exceeds size limit: {}", path.display());
    }
    fs::read_to_string(path).with_context(|| format!("cannot read public key: {}", path.display()))
}

pub(super) fn read_password(variable: &str) -> Result<String> {
    match std::env::var(variable) {
        Ok(password) if !password.is_empty() => Ok(password),
        Ok(_) => bail!("environment variable {variable} holds an empty password"),
        Err(_) => bail!("environment variable {variable} is not set or is not UTF-8"),
    }
}

pub(super) fn decrypt_secret(text: &str, password_env: Option<&str>) -> Result<SecretKey> {
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

pub(super) fn require_directory(path: &Path, role: &str) -> Result<()> {
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

pub(super) fn write_secret_key(path: &Path, bytes: &[u8]) -> Result<()> {
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

pub(super) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
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

pub(super) fn write_signature(path: &Path, bytes: &[u8]) -> Result<()> {
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
