use std::env;
use std::fs;
use std::path::PathBuf;

const OFFICIAL_GAMES: &str = "../../buckets/main/games";

fn main() {
    let games = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join(OFFICIAL_GAMES);
    println!("cargo:rerun-if-changed={}", games.display());

    let mut manifests = Vec::new();
    for game in fs::read_dir(&games).expect("official games directory") {
        let game = game.expect("game entry").path();
        println!("cargo:rerun-if-changed={}", game.display());
        for package in fs::read_dir(&game).expect("official game packages") {
            let path = package.expect("package entry").path();
            assert!(
                path.extension()
                    .is_some_and(|extension| extension == "json"),
                "official bucket entry is not a JSON manifest: {}",
                path.display()
            );
            println!("cargo:rerun-if-changed={}", path.display());
            manifests.push(path);
        }
    }
    manifests.sort();

    let mut source = String::from("[\n");
    for path in &manifests {
        source.push_str(&format!(
            "    include_str!({:?}),\n",
            path.to_str().expect("UTF-8 path")
        ));
    }
    source.push_str("]\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    fs::write(out.join("official_manifests.rs"), source).expect("write official manifest list");
}
