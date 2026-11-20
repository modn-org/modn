//! `modn catalog` subcommands: author key generation, signing and pins.

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
}
