use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn ui_sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect(&Path::new(ROOT).join("ui"), &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect(directory: &Path, files: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(directory).expect("ui directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            let text = fs::read_to_string(&path).expect("ui file");
            files.push((path, text));
        }
    }
}

fn js(files: &[(PathBuf, String)]) -> String {
    files
        .iter()
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "js"))
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn library_composes_the_shared_modules() {
    let library = read("ui/js/library.js");
    for specifier in [
        "./op-button.js",
        "./library/data.js",
        "./library/dom.js",
        "./library/recent.js",
        "./library/tile.js",
    ] {
        assert!(
            library.contains(&format!("from '{specifier}'")),
            "library.js must import {specifier}"
        );
    }
    for command in [
        "catalog(",
        "listPackages(",
        "listInstallations(",
        "pickArchive(",
        "installArchive(",
        "play(",
    ] {
        assert!(
            ui_sources().iter().any(|(_, text)| text.contains(command)),
            "the Library must call {command}"
        );
    }
    let tile = read("ui/js/library/tile.js");
    for command in ["installMod(", "uninstall(", "update(", "undo("] {
        assert!(
            tile.contains(command) || library.contains(command),
            "one-click operations must call {command}"
        );
    }
}

#[test]
fn op_button_handles_progress_cancel_and_approval() {
    let op = read("ui/js/op-button.js");
    for fragment in [
        "'commit'",
        "Finishing...",
        "CANCELLED",
        "APPROVAL_REQUIRED",
        "NATIVE_INJECTOR_NOT_ALLOWED",
        "cancelRun({ runId",
        "onProgress(id",
        "newRunId(",
        "toast-undo",
        "This mod runs native code. Allow it?",
    ] {
        assert!(op.contains(fragment), "op-button.js must handle {fragment}");
    }
    for state in ["idle", "running", "finishing", "blocked"] {
        assert!(
            op.contains(&format!("dataset.state = '{state}'")),
            "op-button.js must expose data-state={state}"
        );
    }
    assert!(
        op.contains("requestCancel") && op.contains("await cancelRun"),
        "the running button must cancel its run"
    );
    assert!(
        op.contains("showUndoToast") && op.contains("undoLatest"),
        "a finished operation must offer Undo"
    );
    assert!(
        op.contains("uncovered.executables"),
        "op-button.js must read uncovered.executables from a refusal or plan"
    );
    let sources = js(&ui_sources());
    assert!(
        sources.contains("allowExecutables: approvals.executables"),
        "the Allow flow must send allowExecutables"
    );
    assert!(
        !sources.contains("allowExecutable: approvals.executable"),
        "no wrapper may still send a boolean allowExecutable"
    );
    let tile = read("ui/js/library/tile.js");
    assert!(
        tile.contains("planInstall("),
        "install and update must read uncovered items from the install plan"
    );
    let data = read("ui/js/library/data.js");
    assert!(
        data.contains("installedPackage") && data.contains("visibleModels"),
        "the grid must merge installed packages and filter by name"
    );
}

#[test]
fn verifier_hooks_are_present() {
    let sources = ui_sources();
    let text = js(&sources);
    for hook in [
        "library-play",
        "library-install-archive",
        "library-recent",
        "recent-undo",
        "filter-all",
        "filter-installed",
        "filter-available",
        "library-search",
        "op-button",
        "ask-allow",
        "ask-decline",
        "toast-undo",
    ] {
        assert!(
            text.contains(&format!("'{hook}'")),
            "missing data-testid {hook}"
        );
    }
    assert!(
        text.contains("`mod-tile-${model.modId}`"),
        "each tile must expose mod-tile-<modId>"
    );
    assert!(
        text.contains("'mod-tile-details'"),
        "each tile must link to Mod details"
    );
}

#[test]
fn library_css_uses_tokens_and_fills_the_grid() {
    let tokens = read("ui/css/tokens.css");
    let mut defined = Vec::new();
    for line in tokens.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--modn-")
            && let Some((name, _)) = rest.split_once(':')
        {
            defined.push(format!("--modn-{name}"));
        }
    }
    let css = read("ui/css/library.css");
    let mut rest = css.as_str();
    while let Some(index) = rest.find("var(--modn-") {
        let after = &rest[index + "var(".len()..];
        let end = after.find(')').expect("closed custom property");
        let usage = &after[..end];
        assert!(
            defined.iter().any(|token| token == usage),
            "library.css uses undefined {usage}"
        );
        rest = &after[end..];
    }
    assert!(
        css.contains("repeat(auto-fill, minmax("),
        "the grid must fill the content width"
    );
    assert!(
        css.contains(".modn-tile {") && css.contains("height: 100%"),
        "tiles in a row must have equal height"
    );
    assert!(
        css.contains(".modn-tile__warning") && css.contains("var(--modn-color-text)"),
        "the warning badge uses the text colour"
    );
}

#[test]
fn catalog_less_installed_packages_stay_in_the_grid() {
    let data = read("ui/js/library/data.js");
    assert!(
        data.contains("const matched = new Set()"),
        "mergeLibrary must remember which packages a catalog entry matched"
    );
    assert!(
        data.contains("for (const [modId, pkg] of installed)"),
        "mergeLibrary must append every installed package the catalog does not list"
    );
    assert!(
        data.contains("if (matched.has(modId))"),
        "only packages without a catalog entry may become extra tiles"
    );
    let start = data
        .find("for (const [modId, pkg] of installed)")
        .expect("unmatched package loop");
    let rest = &data[start..];
    let end = rest.find("for (const pkg of manual)").expect("manual loop");
    let block = &rest[..end];
    assert!(
        block.contains("key: `package-${pkg.id}`"),
        "an unmatched package must get its own stable tile key"
    );
    assert!(
        block.contains("bucket: catalog ? catalog.bucket : ''"),
        "an unmatched catalog package keeps its bucket"
    );
    assert!(
        block.contains("installedPackage: pkg"),
        "the Installed filter and tile actions read installedPackage"
    );
    assert!(
        block.contains("hasUpdate: false"),
        "an unmatched package must not offer Update"
    );
}

#[test]
fn grid_refresh_keeps_running_operation_buttons() {
    let library = read("ui/js/library.js");
    assert!(
        library.contains("const operations = new Map()"),
        "the Library must own one registry of running operation buttons"
    );
    assert!(
        library.contains("createTile(model, ctx, { onChanged: refresh, operations })"),
        "every grid tile must receive the registry"
    );
    let tile = read("ui/js/library/tile.js");
    for fragment in [
        "function operationEntry(operations, key, build)",
        "operations.get(key)",
        "buttonState === 'running'",
        "buttonState === 'finishing'",
        "operations.set(key, entry)",
        "actions.append(entry.button.element)",
        "body.append(entry.askHost)",
    ] {
        assert!(
            tile.contains(fragment),
            "tile.js must reattach a running operation via {fragment}"
        );
    }
}

#[test]
fn edited_files_stay_within_the_size_limit() {
    let mut files = vec![
        Path::new(ROOT).join("ui/js/library.js"),
        Path::new(ROOT).join("ui/js/op-button.js"),
        Path::new(ROOT).join("ui/css/library.css"),
        Path::new(ROOT).join("tests/ui_library.rs"),
    ];
    for (path, _) in ui_sources() {
        if path.starts_with(Path::new(ROOT).join("ui/js/library")) {
            files.push(path);
        }
    }
    for path in files {
        let text = fs::read_to_string(&path).expect("owned file");
        let lines = text.lines().count();
        assert!(lines <= 500, "{path:?} has {lines} lines");
    }
}
