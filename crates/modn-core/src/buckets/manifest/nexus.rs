//! Validation for pinned Nexus Mods package sources.

use super::{ArchiveFormat, PackageManifest};
use anyhow::{bail, Context, Result};

pub(super) fn validate(
    manifest: &PackageManifest,
    game_domain: &str,
    mod_id: u64,
    file_id: u64,
    file_name: &str,
) -> Result<()> {
    crate::acquisition::NexusFile::new(game_domain, mod_id, file_id, file_name)?;
    manifest.required_blake3()?;
    let format = manifest
        .archive_format
        .as_ref()
        .context("archive_format is required for non-git sources")?;
    validate_extension(file_name, format)
}

fn validate_extension(file_name: &str, format: &ArchiveFormat) -> Result<()> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".rar") {
        bail!("Nexus package archives must be .zip or .7z; .rar is not supported");
    }
    let extension = match format {
        ArchiveFormat::Zip => ".zip",
        ArchiveFormat::SevenZ => ".7z",
    };
    if !lower.ends_with(extension) {
        bail!("Nexus file name extension must match archive_format ({extension})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{PackageManifest, PackageSource};
    use serde_json::{json, Value};

    fn parse(source: Value, extra: Value) -> anyhow::Result<PackageManifest> {
        let mut value = json!({
            "schema_version": 2, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0", "archive_format": "zip", "blake3": "0".repeat(64),
            "sources": [source]
        });
        if let (Some(value), Value::Object(extra)) = (value.as_object_mut(), extra) {
            value.extend(extra);
        }
        Ok(serde_json::from_value(value)?)
    }

    fn validate(source: Value, extra: Value) -> anyhow::Result<()> {
        parse(source, extra)?.validate("test-game", "top")
    }

    fn nexus(extra: Value) -> Value {
        let mut source = json!({
            "kind": "nexus", "game_domain": "cyberpunk2077",
            "mod_id": 6945, "file_id": 123456,
            "file_name": "Equipment-EX-1.2.10.zip"
        });
        if let (Some(source), Value::Object(extra)) = (source.as_object_mut(), extra) {
            source.extend(extra);
        }
        source
    }

    #[test]
    fn a_valid_nexus_source_parses_and_validates() {
        let parsed = parse(nexus(json!({})), json!({})).expect("valid source");
        assert!(matches!(
            parsed.sources[0],
            PackageSource::Nexus {
                ref game_domain,
                mod_id: 6945,
                file_id: 123456,
                ref file_name,
            } if game_domain == "cyberpunk2077"
                && file_name == "Equipment-EX-1.2.10.zip"
        ));
        parsed.validate("test-game", "top").expect("valid manifest");
        validate(
            nexus(json!({"file_name": "Mod-1.7z"})),
            json!({"archive_format": "7z"}),
        )
        .expect("7z source");
    }

    #[test]
    fn nexus_domain_ids_and_file_names_are_validated() {
        for (extra, message) in [
            (json!({"game_domain": "Cyberpunk2077"}), "game domain"),
            (json!({"game_domain": "bad domain"}), "game domain"),
            (json!({"game_domain": "bad-"}), "game domain"),
            (json!({"game_domain": "a".repeat(65)}), "game domain"),
            (json!({"mod_id": 0}), "mod ID"),
            (json!({"mod_id": 10_000_000_000u64}), "mod ID"),
            (json!({"file_id": 0}), "file ID"),
            (json!({"file_id": 10_000_000_000u64}), "file ID"),
            (json!({"file_name": ""}), "file name"),
            (json!({"file_name": "bad/name.zip"}), "file name"),
            (json!({"file_name": "bad:name.zip"}), "file name"),
        ] {
            let error = format!(
                "{:#}",
                validate(nexus(extra.clone()), json!({})).unwrap_err()
            );
            assert!(
                error
                    .to_ascii_lowercase()
                    .contains(&message.to_ascii_lowercase()),
                "{extra}: {error}"
            );
        }
    }

    #[test]
    fn nexus_archives_must_be_zip_or_7z_matching_archive_format() {
        let rar = format!(
            "{:#}",
            validate(nexus(json!({"file_name": "Mod.rar"})), json!({})).unwrap_err()
        );
        assert!(rar.contains(".rar"), "{rar}");

        for (source, extra) in [
            (nexus(json!({"file_name": "Mod.7z"})), json!({})),
            (nexus(json!({"file_name": "Mod"})), json!({})),
            (
                nexus(json!({"file_name": "Mod.zip"})),
                json!({"archive_format": "7z"}),
            ),
        ] {
            let error = format!("{:#}", validate(source, extra).unwrap_err());
            assert!(error.contains("extension"), "{error}");
        }
    }

    #[test]
    fn nexus_sources_require_a_digest_and_archive_format() {
        let digest = format!(
            "{:#}",
            validate(nexus(json!({})), json!({"blake3": null})).unwrap_err()
        );
        assert!(digest.contains("blake3 is required"), "{digest}");
        for value in ["0", &"A".repeat(64), &"0".repeat(63)] {
            let error = format!(
                "{:#}",
                validate(nexus(json!({})), json!({"blake3": value})).unwrap_err()
            );
            assert!(error.contains("64 lowercase hex"), "{value}: {error}");
        }

        let format = format!(
            "{:#}",
            validate(nexus(json!({})), json!({"archive_format": null})).unwrap_err()
        );
        assert!(format.contains("archive_format is required"), "{format}");
    }

    #[test]
    fn nexus_sources_reject_credential_fields() {
        for field in ["key", "expires", "user_id", "url", "nxm", "apikey"] {
            let mut source = nexus(json!({}));
            source[field] = json!("secret");
            let error = format!("{:#}", parse(source, json!({})).unwrap_err());
            assert!(error.contains("unknown field"), "{field}: {error}");
            assert!(error.contains(field), "{field}: {error}");
        }
    }
}
