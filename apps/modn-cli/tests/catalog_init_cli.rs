//! `modn catalog init` scaffolds a bucket that validates, signs and resolves.

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog_authoring::{path_text, snapshot, stderr, stdout};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const NAME: &str = "community";
const TITLE: &str = "Community Bucket";
const PACKAGE_ID: &str = "example-mod";
const GAME_ID: &str = "cyberpunk-2077";

const EXPECTED_FILES: [&str; 7] = [
    "bucket.json",
    "PROVENANCE.md",
    "templates/package.example.json",
    ".gitignore",
    ".github/workflows/validate.yml",
    ".github/workflows/autoupdate.yml",
    "mods",
];

struct Area {
    temp: TempDir,
    data: PathBuf,
    state: PathBuf,
}

impl Area {
    fn new() -> Self {
        let temp = TempDir::new().expect("test fixture invariant");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        let state = temp.path().join("state");
        main_bucket::seed_state(&state);
        Self { temp, data, state }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(main_bucket::modn_binary());
        command
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", &self.data)
            .env("MODN_APPDATA", &self.data)
            .arg("--state-dir")
            .arg(&self.state)
            .args(args);
        command.output().expect("run CLI")
    }

    fn scaffold(&self, dir: &Path) -> Output {
        self.run(&[
            "catalog",
            "init",
            path_text(dir),
            "--name",
            NAME,
            "--title",
            TITLE,
        ])
    }
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
}

fn stable_paths(root: &Path) -> Vec<String> {
    snapshot(root).into_iter().map(|(name, _)| name).collect()
}

/// A small ZIP that the sample manifest can pin, and its BLAKE3 pin.
fn sample_archive(temp: &Path) -> (PathBuf, String) {
    use std::io::Write;
    let path = temp.join("example-mod.zip");
    let file = fs::File::create(&path).expect("sample archive");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "archive/pc/mod/example-mod.archive",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("sample archive entry");
    zip.write_all(b"example-payload").expect("sample payload");
    zip.finish().expect("sample archive");
    let pin = blake3::hash(&fs::read(&path).expect("sample archive bytes"))
        .to_hex()
        .to_string();
    (path, pin)
}

/// Put the sample archive where `modn install --dry-run --download` reuses it.
fn seed_cache(area: &Area, archive: PathBuf, url: &str) {
    let url_hash = blake3::hash(url.as_bytes()).to_hex();
    let entry = area.state.join("cache").join(format!(
        "{GAME_ID}#{PACKAGE_ID}#1.0.0#{}.zip",
        &url_hash[..7]
    ));
    fs::create_dir_all(entry.parent().expect("cache directory")).expect("cache directory");
    fs::copy(archive, &entry).expect("cached archive");
}

#[test]
fn init_writes_a_signed_bucket_scaffold_and_lists_next_steps() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    let created = area.scaffold(&catalog);
    assert_success(&created);
    let printed = stdout(&created);

    for relative in EXPECTED_FILES {
        assert!(catalog.join(relative).exists(), "missing {relative}");
        assert!(printed.contains(relative), "output omits {relative}");
    }
    assert_eq!(
        fs::read(catalog.join("bucket.json")).expect("bucket.json"),
        b"{\n  \"schema_version\": 2,\n  \"name\": \"community\",\n  \"title\": \"Community Bucket\"\n}\n"
    );
    let mods = fs::read_dir(catalog.join("mods")).expect("mods directory");
    assert_eq!(mods.count(), 0, "the scaffold must hold no package");

    for step in [
        "modn catalog keygen",
        "mods/<game-id>/<package-id>.json",
        "modn catalog sign",
        "modn catalog verify",
        "Push",
    ] {
        assert!(printed.contains(step), "next steps omit {step}: {printed}");
    }
}

#[test]
fn init_refuses_a_non_empty_directory_and_writes_nothing() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    fs::create_dir_all(&catalog).expect("target directory");
    fs::write(catalog.join(".hidden"), b"keep").expect("hidden file");
    let before = snapshot(&catalog);

    let refused = area.scaffold(&catalog);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("non-empty"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(snapshot(&catalog), before);

    let file = area.temp.path().join("plain-file");
    fs::write(&file, b"keep").expect("plain file");
    let refused = area.scaffold(&file);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("not a directory"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(fs::read(&file).expect("plain file"), b"keep");
}

#[test]
fn init_refuses_invalid_name_and_title_before_writing() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    for name in ["Main", "main", "bad_name", "-leading", "trailing-", ""] {
        let output = area.run(&[
            "catalog",
            "init",
            path_text(&catalog),
            &format!("--name={name}"),
            "--title",
            TITLE,
        ]);
        assert!(!output.status.success(), "accepted name {name:?}");
        assert!(
            stderr(&output).contains("invalid bucket name"),
            "{name:?}: {}",
            stderr(&output)
        );
        assert!(!catalog.exists(), "invalid name {name:?} wrote files");
    }
    let long_title = "a".repeat(129);
    for title in ["", " ", long_title.as_str(), "bad\u{7}title"] {
        let output = area.run(&[
            "catalog",
            "init",
            path_text(&catalog),
            "--name",
            NAME,
            "--title",
            title,
        ]);
        assert!(!output.status.success(), "accepted title {title:?}");
        assert!(
            stderr(&output).contains("invalid bucket title"),
            "{title:?}: {}",
            stderr(&output)
        );
        assert!(!catalog.exists(), "invalid title {title:?} wrote files");
    }
}

#[test]
fn scaffold_keygen_sign_verify_and_register_as_a_local_bucket() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    assert_success(&area.scaffold(&catalog));
    let keys = area.temp.path().join("keys");

    let keygen = area.run(&[
        "catalog",
        "keygen",
        "--out",
        path_text(&keys),
        "--name",
        "author",
    ]);
    assert_success(&keygen);
    let sign = area.run(&[
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
    ]);
    assert_success(&sign);
    let verify = area.run(&[
        "catalog",
        "verify",
        path_text(&catalog),
        "--key",
        path_text(&catalog.join("catalog.pub")),
    ]);
    assert_success(&verify);

    let added = area.run(&["bucket", "add", NAME, path_text(&catalog)]);
    assert_success(&added);
    let listed = area.run(&["--json", "bucket", "list"]);
    assert_success(&listed);
    let buckets: Value = serde_json::from_slice(&listed.stdout).expect("bucket list JSON");
    assert!(
        buckets
            .as_array()
            .expect("bucket list array")
            .iter()
            .any(|bucket| bucket["name"] == NAME && bucket["title"] == TITLE),
        "{buckets}"
    );
    let searched = area.run(&["search", "--json"]);
    assert_success(&searched);
}

#[test]
fn sample_template_becomes_a_package_that_search_and_dry_run_resolve() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    assert_success(&area.scaffold(&catalog));
    let template = fs::read_to_string(catalog.join("templates/package.example.json"))
        .expect("sample manifest template");
    let (archive, pin) = sample_archive(area.temp.path());
    let url =
        "https://github.com/example-owner/example-mod/releases/download/v1.0.0/example-mod.zip";
    let manifest = template
        .replace("<package-id>", PACKAGE_ID)
        .replace("<game-id>", GAME_ID)
        .replace("<Package title>", "Example Mod")
        .replace("<version>", "1.0.0")
        .replace("<64 lowercase hex characters from modn catalog hash>", &pin)
        .replace("<owner>/<repository>", "example-owner/example-mod")
        .replace("<asset-name>", "example-mod");
    let record = catalog.join("mods").join(GAME_ID).join("example-mod.json");
    fs::create_dir_all(record.parent().expect("record parent")).expect("game directory");
    fs::write(&record, manifest).expect("package record");
    seed_cache(&area, archive, url);

    let added = area.run(&["bucket", "add", NAME, path_text(&catalog)]);
    assert_success(&added);
    let searched = area.run(&["search", "--json"]);
    assert_success(&searched);
    let packages: Value = serde_json::from_slice(&searched.stdout).expect("search JSON");
    assert!(
        packages
            .as_array()
            .expect("search array")
            .iter()
            .any(|package| {
                package["bucket"] == NAME
                    && package["game"] == GAME_ID
                    && package["id"] == PACKAGE_ID
            }),
        "{packages}"
    );

    let game = area.temp.path().join("games").join(GAME_ID);
    fs::create_dir_all(game.join("bin/x64")).expect("game marker directory");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"fixture").expect("game marker");
    let dry_run = area.run(&[
        "install",
        &format!("{NAME}/{GAME_ID}/{PACKAGE_ID}"),
        "--download",
        "--dry-run",
        "--path",
        path_text(&game),
    ]);
    assert_success(&dry_run);
    assert!(
        stdout(&dry_run).contains("reviewable"),
        "{}",
        stdout(&dry_run)
    );

    let update_help = area.run(&["bucket", "update", "--help"]);
    assert_success(&update_help);
    assert!(stdout(&update_help).contains("bucket update"));
}

#[test]
fn workflows_run_only_reviewed_commands_and_autoupdate_never_signs() {
    let area = Area::new();
    let catalog = area.temp.path().join("catalog");
    assert_success(&area.scaffold(&catalog));
    let validate = fs::read_to_string(catalog.join(".github/workflows/validate.yml"))
        .expect("validate workflow");
    let autoupdate = fs::read_to_string(catalog.join(".github/workflows/autoupdate.yml"))
        .expect("autoupdate workflow");

    let version = env!("CARGO_PKG_VERSION");
    for required in [
        "on:\n  pull_request:",
        "actions/checkout@v4",
        "secrets.GITHUB_TOKEN",
        "bucket update",
        "bucket add",
        "search --json",
        "install",
        "--download",
        "--dry-run",
        "Cyberpunk2077.exe",
    ] {
        assert!(validate.contains(required), "validate.yml omits {required}");
    }
    assert!(
        validate.contains(&format!(
            "cargo install --locked --git https://github.com/modn-org/modn --tag v{version} modn-cli"
        )),
        "validate.yml does not install the current release"
    );
    assert!(!validate.to_lowercase().contains("catalog verify"));
    assert!(!validate.contains("catalog sign"));

    for required in [
        "on:\n  schedule:",
        "workflow_dispatch:",
        "cron:",
        "contents: write",
        "pull-requests: write",
        "secrets.GITHUB_TOKEN",
        "modn catalog checkver . --update",
        "gh pr create",
        "git push origin \"$branch\"",
    ] {
        assert!(
            autoupdate.contains(required),
            "autoupdate.yml omits {required}"
        );
    }
    assert!(
        !autoupdate.contains(".key"),
        "autoupdate reads a secret key"
    );
    assert!(
        !autoupdate.contains("catalog.pub"),
        "autoupdate touches keys"
    );
    for line in autoupdate.lines() {
        let line = line.trim();
        if line.starts_with("git push") {
            assert!(
                line.contains("\"$branch\""),
                "autoupdate pushes another ref: {line}"
            );
        }
        let command = line.strip_prefix("run: ").unwrap_or(line);
        if command.starts_with("modn ") {
            assert!(
                !command.contains("catalog sign"),
                "autoupdate signs: {command}"
            );
            assert!(
                command == "modn catalog checkver . --update",
                "unreviewed modn command: {command}"
            );
        }
    }
    assert!(!autoupdate.contains("--base main"));
    assert!(!autoupdate.contains("git push origin main"));

    for line in validate.lines() {
        let line = line.trim();
        let command = line.strip_prefix("run: ").unwrap_or(line);
        if let Some(rest) = command.strip_prefix("modn --state-dir \"$state\" ") {
            assert!(
                ["bucket update", "bucket add", "search --json", "install"]
                    .iter()
                    .any(|allowed| rest.starts_with(allowed)),
                "unreviewed modn command: {command}"
            );
        }
    }
}

#[test]
fn init_accepts_an_existing_empty_directory_and_creates_parents() {
    let area = Area::new();
    let empty = area.temp.path().join("empty");
    fs::create_dir(&empty).expect("empty directory");
    assert_success(&area.scaffold(&empty));
    assert!(empty.join("bucket.json").is_file());

    let nested = area.temp.path().join("missing/parents/catalog");
    assert_success(&area.scaffold(&nested));
    assert!(nested.join("mods").is_dir());
    assert_eq!(stable_paths(&nested), stable_paths(&empty));
}
