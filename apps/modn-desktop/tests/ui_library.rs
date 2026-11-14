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
        "renderAsk(",
        "approvalsFrom(",
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
        op.contains("from './library/approvals.js'") && op.contains("from './library/ask.js'"),
        "op-button.js must read approvals and build the ask in its helper modules"
    );
    let approvals = read("ui/js/library/approvals.js");
    assert!(
        approvals.contains("native_injector") && approvals.contains("executables"),
        "the approval helper must read the native injector and executable paths"
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
fn op_button_binds_undo_to_the_operation_it_committed() {
    let op = read("ui/js/op-button.js");
    assert!(
        op.contains("const committedId = operationIdOf(result)"),
        "op-button.js must bind the toast to the result's own operation ID"
    );
    assert!(
        !op.contains("newestOperation"),
        "the toast must not guess an operation from history"
    );
    let binding = read("ui/js/library/undo-target.js");
    assert!(
        binding.contains("result.operationId"),
        "undo-target.js must read the top-level operationId"
    );
    assert!(
        !binding.contains("package_id_before") && !binding.contains("newest("),
        "undo-target.js must not fall back to a history lookup"
    );
    let start = op.find("async function start(").expect("start function");
    let body = &op[start..];
    let resolve = body
        .find("operationIdOf(result)")
        .expect("the finished run resolves its committed operation");
    let undo_toast = body
        .find("showUndoToast(`${active.actionLabel} done`")
        .expect("a bound operation shows an Undo toast");
    let plain_toast = body
        .find("showToast(`${active.actionLabel} done`)")
        .expect("an unbound operation shows a plain toast");
    assert!(
        resolve < undo_toast && undo_toast < plain_toast,
        "the toast must bind its operation before it appears, and omit Undo when no ID is bound"
    );
    assert!(
        op.contains("async function undoLatest(operationId)"),
        "Undo must take the bound operation ID instead of reading the newest history entry"
    );
}

#[test]
fn undo_paths_send_the_approvals_the_ask_collected() {
    let approvals = read("ui/js/library/approvals.js");
    assert!(
        approvals.contains("err.approval"),
        "the approval helper must read the approval a refusal names"
    );
    assert!(
        approvals.contains("refusal.nativeInjector") && approvals.contains("refusal.executables"),
        "a refusal's approval names the native injector and executables directly"
    );
    assert!(
        approvals.contains("if (plan)") && approvals.contains("await plan()"),
        "the plan fills in every capability an incomplete refusal leaves out"
    );
    let op = read("ui/js/op-button.js");
    assert!(
        op.contains("askHost && !active.operationId ? plan : null"),
        "Undo must use only the refusal's approval, never the install plan"
    );
    let tile = read("ui/js/library/tile.js");
    assert!(
        tile.contains("runUndo: ({ runId, operationId, approvals })")
            && tile.contains("allowNativeInjector: approvals.native")
            && tile.contains("allowExecutables: approvals.executables"),
        "the toast Undo must send the approvals the ask collected"
    );
    let recent = read("ui/js/library/recent.js");
    assert!(
        recent.contains("allowNativeInjector: approvals.native")
            && recent.contains("allowExecutables: approvals.executables"),
        "Recent Undo must send the approvals the ask collected"
    );
    assert!(
        recent.contains("askHost") && !recent.contains("askHost: null"),
        "Recent Undo must render the approval ask"
    );
}

#[test]
fn library_filter_tabs_support_keyboard_navigation() {
    let library = read("ui/js/library.js");
    for fragment in [
        "addEventListener('keydown'",
        "'ArrowLeft'",
        "'ArrowRight'",
        "'Home'",
        "'End'",
        ".focus()",
        "'aria-selected'",
        "tab.tabIndex = active ? 0 : -1",
    ] {
        assert!(
            library.contains(fragment),
            "the filter tablist must support {fragment}"
        );
    }
    let tabs = library
        .find("tabs.setAttribute('role', 'tablist')")
        .expect("tablist");
    let toolbar = &library[tabs..];
    let keys = toolbar
        .find("addEventListener('keydown'")
        .expect("arrow-key handler");
    let filter = toolbar.find("function setFilter(").expect("setFilter");
    assert!(
        keys < filter,
        "the tablist must handle arrow keys before the filter is applied"
    );
}

#[test]
fn op_button_reserves_its_operation_before_the_first_await() {
    let op = read("ui/js/op-button.js");
    let start = op.find("async function start(").expect("start function");
    let body = &op[start..];
    let first_await = body.find("await ").expect("start awaits");
    let reserved = body.find("runId = id").expect("the run id is reserved");
    assert!(
        reserved < first_await,
        "start must reserve runId synchronously before any await"
    );
    let head = &body[..first_await];
    assert!(
        head.contains("newRunId(") && head.contains("dataset.state = 'running'"),
        "the reserved operation must exist before prepare runs"
    );
    assert!(
        head.contains("button.disabled = true"),
        "a reserved operation must ignore further clicks"
    );
}

#[test]
fn op_button_stops_a_destroyed_run_before_the_write() {
    let op = read("ui/js/op-button.js");
    let listen = op.find("await onProgress(").expect("progress subscription");
    let write = op[listen..].find("await active.run(").expect("write call") + listen;
    let between = &op[listen..write];
    assert!(
        between.contains("generationAtStart !== generation"),
        "a stale generation must stop the run after the progress subscription"
    );
    assert!(
        between.contains("destroyed"),
        "a destroyed button must stop the run before the write"
    );
    assert!(
        between.contains("off()"),
        "a stale run must drop the progress listener before it starts the write"
    );
    let destroy = op.find("destroy() {").expect("destroy function");
    assert!(
        op[destroy..].contains("generation += 1"),
        "destroy must advance the generation token"
    );
}

#[test]
fn after_commit_failures_are_completed_operations() {
    let messages = read("ui/js/messages.js");
    assert!(
        messages
            .contains("BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT: 'Done. Refresh to see the result.'"),
        "the after-commit code must tell the user to refresh"
    );
    let op = read("ui/js/op-button.js");
    let branch = op
        .find("code === 'BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT'")
        .expect("the completed branch");
    let rest = &op[branch..];
    let end = rest
        .find("} else if (BLOCKED_CODES.has(code))")
        .expect("the blocked branch follows");
    let block = &rest[..end];
    for fragment in [
        "setIdle(active.idleLabel)",
        "showToast(messageFor(code))",
        "onSettled",
    ] {
        assert!(
            block.contains(fragment),
            "an after-commit failure must {fragment} like a completed operation"
        );
    }
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
        data.contains("const installed = new Map()"),
        "mergeLibrary must group installed packages by mod ID"
    );
    assert!(
        data.contains("installed.set(modId, [pkg])"),
        "a second package of the same mod ID must join the first"
    );
    assert!(
        data.contains("for (const [modId, list] of installed)"),
        "mergeLibrary must append every package the catalog rows did not bind"
    );
    assert!(
        data.contains("if (!matched.has(pkg))"),
        "only packages without a catalog row may become extra tiles"
    );
    let start = data
        .find("for (const [modId, list] of installed)")
        .expect("unmatched package loop");
    let rest = &data[start..];
    let end = rest.find("for (const pkg of manual)").expect("manual loop");
    let block = &rest[..end];
    assert!(
        block.contains("packageModel(pkg, modId)"),
        "an unmatched package keeps its package and bucket"
    );
    let builder = data
        .find("function packageModel(")
        .expect("the package row builder");
    let model = &data[builder..];
    for fragment in [
        "key: `package-${pkg.id}`",
        "bucket: catalog ? catalog.bucket : ''",
        "installedPackage: pkg",
        "hasUpdate: false",
    ] {
        assert!(
            model.contains(fragment),
            "a package row must set {fragment}"
        );
    }
}

#[test]
fn grid_refresh_keeps_running_buttons_and_live_toast_undo() {
    let library = read("ui/js/library.js");
    assert!(
        library.contains("const operations = new Map()"),
        "the Library must own one registry of running operation buttons"
    );
    assert!(
        library.contains("createTile(model, ctx, { onChanged: refresh, operations, undoHost })"),
        "every grid tile must receive the registry and undo host"
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
    let op = read("ui/js/op-button.js");
    assert!(op.contains("launchUndo(committedId)"), "toast runner");
    let launcher = tile.find("createUndoLauncher").expect("launcher");
    let runner = &tile[launcher..];
    assert!(runner.contains("createOperationButton("), "runner button");
    assert!(runner.contains("askHost,"), "runner ask");
    assert!(runner.contains("void runner.start("), "runner start");
    assert!(!runner.contains("operations.set("), "runner not grid");
    assert!(library.contains("const undoHost"), "library host");
    assert!(tile.contains("launchUndo: undoToast"), "tile runner");
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
