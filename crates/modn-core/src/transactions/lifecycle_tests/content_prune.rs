use crate::state::Store;
use crate::transactions::ManagedPackage;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const MODULE_LUA: &str = r#"return {
    game_id="content-game", api_version=2, version="1.0.0",
    marker="ContentGame.exe", common_paths={},
    validate=function() return true end,
    steam_name=function() return "Content Game" end,
    is_epic=function() return false end,
    scan=function() return {mods={},framework_signals={},incomplete=false,redmod_available=false} end,
    doctor=function() return {} end,
    plan=function(files)
        local operations={}
        for _, file in ipairs(files) do
            local game_path=file.source:match("^game/(.+)$")
            if game_path then
                table.insert(operations,{source=file.source,destination=game_path,kind="game-mod",package_root="",root="game"})
            else
                table.insert(operations,{source=file.source,destination=file.source,kind="content-mod",package_root="",root="content"})
            end
        end
        return {schema_version=1,operations=operations,blockers={}}
    end,
}"#;

struct Lab {
    temp: TempDir,
    game: PathBuf,
    appdata: PathBuf,
    content: PathBuf,
    module: PathBuf,
    store: Store,
}

impl Lab {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("ContentGame.exe"), b"marker").unwrap();
        let appdata = temp.path().join("appdata");
        let content = appdata.join("Fixture/Mods");
        fs::create_dir_all(&content).unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(&module).unwrap();
        fs::write(
            module.join("module.json"),
            r#"{"game_id":"content-game","version":"1.0.0","api_version":2,"entrypoint":"module.lua","content_root":{"base":"appdata","path":"Fixture/Mods"}}"#,
        )
        .unwrap();
        fs::write(module.join("module.lua"), MODULE_LUA).unwrap();
        let store = Store::open(&temp.path().join("state/state.sqlite")).unwrap();
        Self {
            temp,
            game,
            appdata,
            content,
            module,
            store,
        }
    }

    fn install(&mut self, game: &Path, entries: &[&str]) -> anyhow::Result<ManagedPackage> {
        let archive = self.temp.path().join("mod.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        for name in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let module = self.module.clone();
        crate::game_modules::with_module_source(Some(&module), || {
            let plan = crate::planning::preview_install_zip_with_appdata(
                "content-game",
                game,
                &archive,
                &self.appdata,
            )?;
            self.store.install_operations_zip(game, &archive, &plan)
        })
    }

    fn install_and_uninstall(&mut self, entries: &[&str], unowned: &[&Path]) {
        let game = self.game.clone();
        let package = self.install(&game, entries).unwrap();
        assert_eq!(package.files.len(), entries.len());
        assert_eq!(self.created(), entries.len());
        for path in unowned {
            fs::write(path, b"user").unwrap();
        }
        self.store.uninstall(&game, &package.id).unwrap();
    }

    fn created(&self) -> usize {
        self.store
            .connection
            .query_row("SELECT COUNT(*) FROM created_directories", [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}

#[test]
fn uninstall_prunes_created_folders_in_the_game_and_content_roots() {
    let mut lab = Lab::new();
    lab.install_and_uninstall(&["game/g/a.lua", "c/b.lua"], &[]);
    assert!(!lab.game.join("g").exists());
    assert!(!lab.content.join("c").exists());
    assert!(lab.content.is_dir());
    assert_eq!(lab.created(), 0);
}

#[test]
fn uninstall_keeps_created_folders_that_hold_unowned_files_in_each_root() {
    let mut lab = Lab::new();
    let game_user = lab.game.join("h/user.txt");
    let content_user = lab.content.join("d/user.txt");
    lab.install_and_uninstall(
        &["game/g/a.lua", "game/h/b.lua", "c/a.lua", "d/b.lua"],
        &[&game_user, &content_user],
    );
    assert!(!lab.game.join("g").exists());
    assert!(!lab.content.join("c").exists());
    assert!(!lab.game.join("h/b.lua").exists());
    assert!(!lab.content.join("d/b.lua").exists());
    assert_eq!(fs::read(&game_user).unwrap(), b"user");
    assert_eq!(fs::read(&content_user).unwrap(), b"user");
    let mut statement = lab
        .store
        .connection
        .prepare("SELECT root, relative_path FROM created_directories ORDER BY root")
        .unwrap();
    let kept: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        kept,
        [
            ("content".to_owned(), "d".to_owned()),
            ("game".to_owned(), "h".to_owned())
        ]
    );
}

#[test]
fn a_second_installation_cannot_claim_a_recorded_content_root() {
    let mut lab = Lab::new();
    let first = lab.game.clone();
    lab.install(&first, &["c/a.lua"]).unwrap();
    let second = lab.temp.path().join("second");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("ContentGame.exe"), b"marker").unwrap();
    let state = fs::read(lab.temp.path().join("state/state.sqlite")).unwrap();

    let error = lab
        .install(&second, &["game/g/a.lua", "d/b.lua"])
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("two installations cannot share a root"),
        "{error:#}"
    );
    assert!(lab.store.installed(&second).unwrap().is_empty());
    assert!(!second.join("g").exists());
    assert!(!lab.content.join("d").exists());
    assert_eq!(fs::read(lab.content.join("c/a.lua")).unwrap(), b"c/a.lua");
    assert_eq!(
        fs::read(lab.temp.path().join("state/state.sqlite")).unwrap(),
        state
    );
}
