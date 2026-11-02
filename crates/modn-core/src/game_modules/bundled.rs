use super::loader::{self, VerifiedModule};
use anyhow::{Context, Result};

struct BundledModule {
    game_id: &'static str,
    manifest: &'static str,
    entrypoint: &'static str,
    script: &'static str,
}

const BUNDLED: &[BundledModule] = &[BundledModule {
    game_id: "cyberpunk-2077",
    manifest: include_str!("../../game-modules/cyberpunk-2077/module.json"),
    entrypoint: "main.lua",
    script: include_str!("../../game-modules/cyberpunk-2077/main.lua"),
}];

pub(super) fn read_bundled(game: &str) -> Result<(VerifiedModule, String)> {
    let module = BUNDLED
        .iter()
        .find(|module| module.game_id == game)
        .with_context(|| {
            format!("no first-party game module for {game}; select a development source with --module-src")
        })?;
    loader::verify_embedded(
        module.manifest.as_bytes(),
        module.entrypoint,
        module.script.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::{read_bundled, BUNDLED};
    use std::fs;
    use std::path::Path;

    #[test]
    fn every_source_module_is_embedded_and_verifies() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("game-modules");
        let mut sources: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        sources.sort();
        let mut embedded: Vec<&str> = BUNDLED.iter().map(|module| module.game_id).collect();
        embedded.sort_unstable();
        assert_eq!(
            sources, embedded,
            "add new game-modules directories to BUNDLED"
        );
        for game in embedded {
            let (manifest, source) = read_bundled(game).unwrap();
            assert_eq!(manifest.game_id, game);
            let on_disk = fs::read_to_string(root.join(game).join(&manifest.entrypoint)).unwrap();
            assert_eq!(source, on_disk);
        }
    }

    #[test]
    fn unknown_game_has_no_bundled_module() {
        let error = read_bundled("unknown-game").unwrap_err().to_string();
        assert!(error.contains("no first-party game module"), "{error}");
    }
}
