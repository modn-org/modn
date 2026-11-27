use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum CatalogCommand {
    /// Create a minisign key pair that signs catalogs.
    Keygen {
        /// Directory that receives <name>.key and <name>.pub.
        #[arg(long, value_name = "DIR", default_value = ".")]
        out: PathBuf,
        /// Base name of the key files.
        #[arg(long, default_value = "catalog")]
        name: String,
        /// Environment variable that holds the password of the secret key.
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
    },
    /// Write catalog.sig and catalog.pub for a catalog directory.
    Sign {
        /// Catalog directory to sign.
        dir: PathBuf,
        /// Secret key file that `modn catalog keygen` wrote.
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
        /// Environment variable that holds the password of the secret key.
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
        /// Public key file to publish as the offline revocation key.
        #[arg(long, value_name = "FILE")]
        revocation_public_key: Option<PathBuf>,
        /// Sequence number to write to sequence.json: a number of at least 1, or "next".
        #[arg(long, value_name = "N|next")]
        sequence: Option<String>,
        /// UTC expiry timestamp to write to sequence.json, such as 2027-12-31T00:00:00Z.
        #[arg(long, value_name = "TIME")]
        expires: Option<String>,
    },
    /// Write rotation.json and rotation.sig, signed by the current signing key.
    Rotate {
        /// Catalog directory to update.
        dir: PathBuf,
        /// Current signing secret key file, which must live outside the catalog.
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
        /// Public key file of the successor signing key.
        #[arg(long, value_name = "FILE")]
        new_public_key: PathBuf,
        /// Environment variable that holds the password of the secret key.
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
        /// Statement sequence, which must equal sequence.json when it exists.
        #[arg(long, value_name = "N")]
        sequence: Option<u64>,
    },
    /// Write revocation.json and revocation.sig, signed by the offline revocation key.
    Revoke {
        /// Catalog directory to update.
        dir: PathBuf,
        /// Offline revocation secret key file, which must live outside the catalog.
        #[arg(long, value_name = "FILE")]
        revocation_key: PathBuf,
        /// Fingerprint of the signing key that this statement revokes.
        #[arg(long, value_name = "KEY_ID")]
        revoked: String,
        /// Public key file of the replacement signing key.
        #[arg(long, value_name = "FILE")]
        replacement_public_key: PathBuf,
        /// Environment variable that holds the password of the secret key.
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
        /// Statement sequence, which must equal sequence.json when it exists.
        #[arg(long, value_name = "N")]
        sequence: Option<u64>,
    },
    /// Verify a catalog against a public key without registering it.
    Verify {
        /// Catalog directory to verify.
        dir: PathBuf,
        /// Public key file, such as the catalog's own catalog.pub.
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
    },
    /// Print the BLAKE3 pin of an archive for its package record.
    Hash {
        /// Archive file, such as a ZIP.
        archive: PathBuf,
    },
    /// Scaffold a contract v2 bucket repository with a sample manifest and CI.
    Init {
        /// Empty or missing directory that receives the bucket.
        dir: PathBuf,
        /// Bucket name, lowercase letters, digits and dashes; not "main".
        #[arg(long)]
        name: String,
        /// Human-readable bucket title, at most 128 bytes without control characters.
        #[arg(long)]
        title: String,
    },
    /// Report whether each catalog package has a newer upstream release.
    Checkver {
        /// Catalog directory to check.
        dir: PathBuf,
        /// One package: <mod-id> or <game-id>/<mod-id>.
        #[arg(value_name = "MOD")]
        mod_id: Option<String>,
        /// Download each newer upstream release and rewrite its package manifest.
        #[arg(long)]
        update: bool,
        /// Save the replaced manifest as <mod-id>@<old-version>.json.
        #[arg(long, requires = "update")]
        keep_previous: bool,
    },
}
