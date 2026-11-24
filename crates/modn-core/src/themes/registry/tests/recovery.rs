use super::super::layout::Failpoint;
use super::support::{digest, fixture, theme_source, write, Accept, BASE_CSS, GAME, ID};
use crate::themes::{
    CheckedCss, CssChecker, CssContext, Rule, ThemeError, ThemeProblem, ThemeRegistry,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn registry_root(temp: &TempDir) -> PathBuf {
    temp.path().join("data/themes/registry")
}

fn live_root(temp: &TempDir) -> PathBuf {
    registry_root(temp).join(GAME).join(ID)
}

fn backup_root(temp: &TempDir) -> PathBuf {
    registry_root(temp).join(".backup").join(GAME).join(ID)
}

fn theme_copy(entry: &Path) -> PathBuf {
    entry.join("theme")
}

fn has_staging(root: &Path) -> bool {
    fs::read_dir(root).is_ok_and(|mut entries| {
        entries.any(|entry| {
            entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with(".staging-"))
        })
    })
}

#[test]
fn an_interrupted_replacement_restores_the_previous_theme_on_list() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    let old_digest = first.blake3().to_owned();
    write(&source.join("theme.css"), b".modn-app { color: #202020; }");
    registry.fail_at(Failpoint::AfterBackup);
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(matches!(error, ThemeError::Io { .. }), "{error}");
    assert!(!live_root(&temp).exists());
    assert!(backup_root(&temp).is_dir());
    assert_eq!(digest(&theme_copy(&backup_root(&temp))), old_digest);
    let fresh = ThemeRegistry::new(&temp.path().join("data"));
    let listed = fresh.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].blake3(), old_digest);
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
    assert_eq!(fresh.load(GAME, ID, &Accept).unwrap().css, BASE_CSS);
    assert!(!backup_root(&temp).exists());
}

#[test]
fn a_replacement_interrupted_after_install_keeps_the_new_theme() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    write(&source.join("theme.css"), b".modn-app { color: #303030; }");
    registry.fail_at(Failpoint::BeforeBackupDelete);
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(matches!(error, ThemeError::Io { .. }), "{error}");
    assert!(live_root(&temp).is_dir());
    assert!(backup_root(&temp).is_dir());
    let new_digest = digest(&theme_copy(&live_root(&temp)));
    assert_ne!(new_digest, first.blake3());
    let fresh = ThemeRegistry::new(&temp.path().join("data"));
    let listed = fresh.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].blake3(), new_digest);
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
    assert!(!backup_root(&temp).exists());
}

#[test]
fn a_failed_install_restores_the_previous_registration() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    write(&source.join("theme.css"), b".modn-app { color: #404040; }");
    registry.fail_at(Failpoint::InstallFails);
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(matches!(error, ThemeError::Io { .. }), "{error}");
    assert!(live_root(&temp).is_dir());
    assert!(!backup_root(&temp).exists());
    let loaded = registry.load(GAME, ID, &Accept).unwrap();
    assert_eq!(loaded.css, BASE_CSS);
    assert_eq!(loaded.digest, first.blake3());
}

#[test]
fn a_failed_restore_keeps_the_backup_and_names_it() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    let old_digest = first.blake3().to_owned();
    write(&source.join("theme.css"), b".modn-app { color: #505050; }");
    registry.fail_at(Failpoint::InstallAndRestoreFail);
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(!live_root(&temp).exists());
    assert!(backup_root(&temp).is_dir());
    assert_eq!(digest(&theme_copy(&backup_root(&temp))), old_digest);
    let message = error.to_string().replace('\\', "/");
    let expected = backup_root(&temp).to_string_lossy().replace('\\', "/");
    assert!(message.contains(&expected), "{message}");
    let listed = ThemeRegistry::new(&temp.path().join("data"))
        .list()
        .unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].blake3(), old_digest);
}

#[test]
fn register_removes_stale_staging_and_restores_backups() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    let old_digest = first.blake3().to_owned();
    write(&source.join("theme.css"), b".modn-app { color: #606060; }");
    registry.fail_at(Failpoint::AfterBackup);
    registry.register(&source, &Accept).unwrap_err();
    write(
        &registry_root(&temp).join(".staging-crashed/theme/theme.css"),
        b"stale",
    );
    // A backup that recovery keeps because its live copy is broken must
    // survive both the next register and the staging cleanup.
    let kept = theme_source(&temp, GAME, "kept-theme", "Kept");
    let kept_registered = registry.register(&kept, &Accept).unwrap();
    write(
        &kept_registered.dir().join("theme.css"),
        b".modn-app { color: #777777; }",
    );
    let kept_backup = registry_root(&temp)
        .join(".backup")
        .join(GAME)
        .join("kept-theme");
    write(&kept_backup.join("theme/theme.css"), b"kept");
    let other = theme_source(&temp, GAME, "day-city", "Day City");
    registry.register(&other, &Accept).unwrap();
    assert!(!registry_root(&temp).join(".staging-crashed").exists());
    assert!(!has_staging(&registry_root(&temp)));
    assert!(kept_backup.is_dir());
    assert!(live_root(&temp).is_dir());
    assert_eq!(digest(&theme_copy(&live_root(&temp))), old_digest);
    assert!(!backup_root(&temp).exists());
    assert_eq!(registry.list().unwrap().themes.len(), 2);
}

struct RewritingChecker {
    source: PathBuf,
}

impl CssChecker for RewritingChecker {
    fn check(
        &self,
        _file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        write(
            &self.source.join("theme.css"),
            b".modn-app { color: #badbad; }",
        );
        Ok(CheckedCss::new(css.to_owned(), Vec::new()))
    }
}

#[test]
fn a_source_rewritten_during_inspection_does_not_change_the_installed_copy() {
    let (temp, registry, source) = fixture();
    let checker = RewritingChecker {
        source: source.clone(),
    };
    let registered = registry.register(&source, &checker).unwrap();
    assert_eq!(
        fs::read(registered.dir().join("theme.css")).unwrap(),
        BASE_CSS.as_bytes()
    );
    assert_eq!(registered.blake3(), digest(registered.dir()));
    assert_eq!(
        fs::read(source.join("theme.css")).unwrap(),
        b".modn-app { color: #badbad; }"
    );
    assert_eq!(registry.load(GAME, ID, &Accept).unwrap().css, BASE_CSS);
    assert!(temp.path().join("data/themes/registry").is_dir());
}

#[test]
fn register_refuses_a_copy_that_exceeds_the_byte_limit() {
    let (temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    let old_digest = first.blake3().to_owned();
    write(
        &source.join("assets/huge.bin"),
        &vec![0u8; 16 * 1024 * 1024 + 1],
    );
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(
        matches!(&error, ThemeError::Invalid(problems)
            if problems.iter().any(|problem| problem.rule == Rule::ThemeTooLarge)),
        "{error}"
    );
    assert!(!has_staging(&registry_root(&temp)));
    assert_eq!(registry.load(GAME, ID, &Accept).unwrap().digest, old_digest);
}

#[test]
fn register_refuses_a_copy_with_too_many_entries() {
    let (temp, registry, source) = fixture();
    for index in 0..2049 {
        write(&source.join(format!("assets/f{index:04}.bin")), b"x");
    }
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(
        matches!(&error, ThemeError::Invalid(problems)
            if problems.iter().any(|problem| problem.rule == Rule::ThemeTooLarge)),
        "{error}"
    );
    assert!(!has_staging(&registry_root(&temp)));
    assert!(!live_root(&temp).exists());
}
