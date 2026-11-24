"""The `themes` desktop journey for the Modn desktop verifier.

The journey builds five fixture themes and proves theme API v1 in the Theme
control: the official fallback, the `bold` variant, a locally registered
theme, a community bucket theme, the broken-theme diagnostic, the
digest-mismatch fallback and the default reset. Step 7 is limited to the CLI
refusal of the protected theme; the install review is not opened at runtime.
"""

import json
import time

import uia
from fixtures import Fixture
from lifecycle import GAME_ID, _app, _assert_clean_ui, _cli, _cli_path, _open_game
from theme_fixture import (
    build_community_bucket,
    build_local_theme,
    build_main_with_official,
    build_protected_theme,
    expected_order,
    taller,
)

OFFICIAL = "fixture-official"
OFFICIAL_NAME = "Fixture Official"
OFFICIAL_BOLD = "fixture-official/bold"
LOCAL = "fixture-local"
LOCAL_NAME = "Fixture Local"
COMMUNITY = "fixture-community"
COMMUNITY_NAME = "Fixture Community"
BROKEN = "fixture-broken"
BROKEN_NAME = "Fixture Broken"
BROKEN_RULE = "forbidden-position-fixed"
PROTECTED_RULE = "protected-region"
DIGEST_RULE = "digest-mismatch"


# -- small helpers ---------------------------------------------------------


def _expect_ok(runner, step, result, context):
    runner.check(
        result.returncode == 0,
        f"step {step}: {context} failed with status {result.returncode}: "
        f"{result.stderr.strip()}",
    )
    return result


def _payload(runner, step, result, context):
    _expect_ok(runner, step, result, context)
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        runner.check(False, f"step {step}: {context} printed no JSON: {result.stdout.strip()!r}")
        return {}


def _json_or_empty(text):
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {}


def _entries(listed):
    return {entry.get("id"): entry for entry in listed.get("themes", [])}


def _source(entry):
    source = entry.get("source") or {}
    return source.get("kind"), source.get("label")


def _wait(runner, step, session, predicate, description, timeout=90):
    try:
        session.wait_for(predicate, description, timeout=timeout)
    except uia.Failure as error:
        runner.check(False, f"step {step}: {error}")


def _theme_button(runner, step, session):
    buttons = session.find_all(name="Theme", control_type="ButtonControl")
    runner.check(len(buttons) == 1, f"step {step}: expected one Theme button, found {len(buttons)}")
    return buttons[0]


def _height(control):
    return control.BoundingRectangle.height()


def _wait_height(runner, step, session, accept, description, timeout=60):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        buttons = session.find_all(name="Theme", control_type="ButtonControl")
        if buttons:
            last = _height(buttons[0])
            if accept(last):
                return last
        time.sleep(0.2)
    runner.check(False, f"step {step}: timed out waiting for {description}; last height was {last}")
    return last


def _open_panel(runner, step, session):
    _theme_button(runner, step, session)
    session.click(name="Theme", control_type="ButtonControl")
    _wait(
        runner,
        step,
        session,
        lambda current: current.exists(name="Use theme"),
        "the Theme control panel",
        timeout=60,
    )


def _ensure_panel(runner, step, session):
    if not session.exists(name="Use theme"):
        _open_panel(runner, step, session)


def _picks(session, name):
    return session.find_all(prefix=name, control_type="ButtonControl")


def _choose(runner, step, session, name):
    controls = _picks(session, name)
    runner.check(bool(controls), f"step {step}: the Theme control has no {name} choice")
    session.activate(controls[0])


def _selected(session, name):
    return any("Selected" in (control.Name or "") for control in _picks(session, name))


def _apply(session):
    session.click(name="Use theme", control_type="ButtonControl")


def _choose_variant(runner, step, session, variant):
    combos = session.find_all(control_type="ComboBoxControl")
    runner.check(bool(combos), f"step {step}: the Theme control has no variant selector")
    combo = next((entry for entry in combos if "Variant" in (entry.Name or "")), combos[0])
    try:
        combo.GetValuePattern().SetValue(variant)
        return
    except Exception:
        pass
    try:
        combo.GetExpandCollapsePattern().Expand()
        _wait(
            runner,
            step,
            session,
            lambda current: current.find_all(
                name=variant, control_type="ListItemControl"
            ),
            f"the {variant} variant option",
            timeout=30,
        )
        session.activate(session.find_all(name=variant, control_type="ListItemControl")[0])
    except uia.Failure:
        raise
    except Exception as error:
        runner.check(False, f"step {step}: could not choose the {variant} variant: {error}")


def _resolve(fx):
    result = _cli(fx, "theme", "resolve", "--game", GAME_ID)
    if result.returncode != 0:
        return None
    return _json_or_empty(result.stdout)


def _wait_resolve(runner, step, fx, predicate, description, timeout=90):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = _resolve(fx)
        if last and predicate(last):
            return last
        time.sleep(0.2)
    runner.check(
        False,
        f"step {step}: timed out waiting for {description}; "
        f"last resolve was {last}",
    )
    return None


def _applied(runner, step, fx, theme_id, variant=None, source_label=None):
    def matches(resolved):
        theme = resolved.get("theme") or {}
        if theme.get("id") != theme_id or theme.get("origin") != "selected":
            return False
        if variant is not None and theme.get("variant") != variant:
            return False
        if source_label is not None and (theme.get("source") or {}).get("label") != source_label:
            return False
        return resolved.get("fallback") is None

    return _wait_resolve(runner, step, fx, matches, f"{theme_id} to be applied as selected")


def _assert_growth(runner, step, earlier_px, later_px, earlier_key, later_key):
    runner.check(
        expected_order([earlier_key, later_key]) == [earlier_key, later_key],
        f"step {step}: fixture heights do not order {earlier_key} before {later_key}",
    )
    runner.check(
        taller(later_px, earlier_px),
        f"step {step}: {later_key} did not grow past {earlier_key} "
        f"({later_px} vs {earlier_px} px)",
    )


# -- the seven steps -------------------------------------------------------


def _step1_cli(runner, fx, protected):
    listed = _payload(runner, 1, _cli(fx, "theme", "list", "--game", GAME_ID), "theme list")
    entries = _entries(listed)
    sources = (
        (OFFICIAL, "official", "main bucket"),
        (LOCAL, "local", "local registry"),
        (COMMUNITY, "bucket", "community"),
    )
    for theme_id, kind, label in sources:
        entry = entries.get(theme_id)
        runner.check(
            entry is not None,
            f"step 1: theme list does not show {theme_id}: {sorted(entries)}",
        )
        if entry is not None:
            observed = _source(entry)
            runner.check(
                observed == (kind, label),
                f"step 1: {theme_id} source is {observed[0]}/{observed[1]}, "
                f"not {kind}/{label}",
            )
    runner.check(BROKEN not in entries, f"step 1: theme list offers the broken theme {BROKEN}")
    rules = [item.get("rule") for item in listed.get("diagnostics", [])]
    runner.check(
        BROKEN_RULE in rules,
        f"step 1: theme list diagnostics do not name {BROKEN_RULE}: {rules}",
    )

    checked = _cli(fx, "theme", "check", str(protected))
    runner.check(checked.returncode != 0, "step 1: theme check accepted the protected theme")
    refusal = _json_or_empty(checked.stdout)
    rules = [item.get("rule") for item in refusal.get("diagnostics", [])]
    runner.check(
        PROTECTED_RULE in rules,
        f"step 1: the protected refusal does not name {PROTECTED_RULE}: {rules}",
    )
    _expect_ok(runner, 1, _cli(fx, "bucket", "list"), "bucket list")
    _expect_ok(runner, 1, _cli(fx, "list"), "list")


def _step2_default(runner, session):
    _open_panel(runner, 2, session)
    _wait(
        runner,
        2,
        session,
        lambda current: current.find_all(
            prefix=OFFICIAL_NAME, control_type="ButtonControl"
        ),
        f"the {OFFICIAL_NAME} choice",
        timeout=60,
    )
    text = session.text()
    runner.check(OFFICIAL_NAME in text, f"step 2: the Theme control does not show {OFFICIAL_NAME}")
    runner.check(
        "main bucket" in text,
        "step 2: the Theme control does not show the main bucket source",
    )
    base_height = _height(_theme_button(runner, 2, session))
    runner.check(base_height > 0, "step 2: the Theme button has no measured height")
    return base_height


def _step3_variant(runner, session, fx, base_height):
    _choose(runner, 3, session, OFFICIAL_NAME)
    _choose_variant(runner, 3, session, "bold")
    _apply(session)
    _applied(runner, 3, fx, OFFICIAL, variant="bold", source_label="main bucket")
    _wait(
        runner,
        3,
        session,
        lambda current: _selected(current, OFFICIAL_NAME),
        "the bold choice to be marked Selected",
    )
    bold_height = _wait_height(
        runner, 3, session, lambda px: taller(px, base_height), "the bold button to grow"
    )
    _assert_growth(runner, 3, base_height, bold_height, OFFICIAL, OFFICIAL_BOLD)
    _assert_clean_ui(runner, session, "themes-3-bold")
    return bold_height


def _step4_themes(runner, session, fx, bold_height):
    _choose(runner, 4, session, LOCAL_NAME)
    _apply(session)
    _applied(runner, 4, fx, LOCAL, source_label="local registry")
    local_height = _wait_height(
        runner, 4, session, lambda px: taller(px, bold_height), "the local theme button to grow"
    )
    _assert_growth(runner, 4, bold_height, local_height, OFFICIAL_BOLD, LOCAL)

    _choose(runner, 4, session, COMMUNITY_NAME)
    _apply(session)
    _applied(runner, 4, fx, COMMUNITY, source_label="community")
    community_height = _wait_height(
        runner,
        4,
        session,
        lambda px: taller(px, local_height),
        "the community theme button to grow",
    )
    _assert_growth(runner, 4, local_height, community_height, LOCAL, COMMUNITY)
    text = session.text()
    runner.check(
        LOCAL_NAME in text and "local registry" in text,
        "step 4: the Theme control does not show the local theme source",
    )
    runner.check(
        COMMUNITY_NAME in text and "community" in text,
        "step 4: the Theme control does not show the community bucket source",
    )
    _assert_clean_ui(runner, session, "themes-4-community")
    return community_height


def _step5_refusal(runner, app, fx, session):
    _ensure_panel(runner, 5, session)
    _wait(
        runner,
        5,
        session,
        lambda current: BROKEN_RULE in current.text(),
        f"the {BROKEN_RULE} diagnostic",
        timeout=60,
    )
    runner.check(
        not session.exists(prefix=BROKEN_NAME, control_type="ButtonControl"),
        f"step 5: the Theme control offers {BROKEN_NAME}",
    )

    _expect_ok(
        runner,
        5,
        _cli(fx, "theme", "select", GAME_ID, LOCAL),
        "theme select fixture-local",
    )
    app.kill()
    css = fx.state / "themes" / "registry" / GAME_ID / LOCAL / "theme" / "theme.css"
    runner.check(css.is_file(), f"step 5: the registered local theme has no {css}")
    if css.is_file():
        payload = css.read_bytes()
        runner.check(bool(payload), "step 5: the registered theme.css is empty")
        if payload:
            css.write_bytes(payload[:-1] + bytes([payload[-1] ^ 1]))

    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")
    _ensure_panel(runner, 5, session)
    _wait(
        runner,
        5,
        session,
        lambda current: DIGEST_RULE in current.text(),
        f"the {DIGEST_RULE} diagnostic",
    )
    text = session.text()
    runner.check(OFFICIAL_NAME in text, f"step 5: the fallback does not show {OFFICIAL_NAME}")
    runner.check("main bucket" in text, "step 5: the fallback does not show the main bucket source")
    _assert_clean_ui(runner, session, "themes-5-fallback")
    return app, session


def _step6_default(runner, session, fx, official_height, local_height):
    _ensure_panel(runner, 6, session)
    session.click(name="Use default theme", control_type="ButtonControl")
    _wait_resolve(
        runner,
        6,
        fx,
        lambda resolved: resolved.get("theme") is None,
        "theme resolve to report the default theme",
    )
    _wait(
        runner,
        6,
        session,
        lambda current: "Default theme" in current.text(),
        "the Default theme label",
        timeout=60,
    )
    default_height = _wait_height(
        runner,
        6,
        session,
        lambda px: not taller(px, official_height),
        "the default button height",
    )
    runner.check(
        not taller(default_height, official_height),
        "step 6: the default control is taller than the official base "
        f"({default_height} vs {official_height} px)",
    )
    runner.check(
        taller(local_height, default_height),
        "step 6: the default control did not shrink below the local theme "
        f"({default_height} vs {local_height} px)",
    )
    _assert_clean_ui(runner, session, "themes-6-default")


def _step7_protected(runner, fx, protected):
    checked = _cli(fx, "theme", "check", str(protected))
    runner.check(checked.returncode != 0, "step 7: theme check accepted the protected theme")
    refusal = _json_or_empty(checked.stdout)
    rules = [item.get("rule") for item in refusal.get("diagnostics", [])]
    runner.check(
        PROTECTED_RULE in rules,
        f"step 7: the protected refusal does not name {PROTECTED_RULE}: {rules}",
    )
    _expect_ok(
        runner,
        7,
        _cli(fx, "theme", "select", GAME_ID, COMMUNITY),
        "theme select fixture-community",
    )
    resolved = _wait_resolve(
        runner,
        7,
        fx,
        lambda answer: (answer.get("theme") or {}).get("id") == COMMUNITY,
        "the community theme to resolve as selected",
    )
    runner.check(
        (resolved.get("theme") or {}).get("origin") == "selected",
        "step 7: the resolved theme is not selected",
    )


def journey_themes(runner):
    """12. Theme control: official fallback, variant, local and bucket themes, diagnostics."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    main = build_main_with_official(fx.data / "modn" / "buckets" / "main")
    runner.check(main.is_dir(), "The official fixture bucket was not written")
    community = build_community_bucket(fx.root / "community")
    local = build_local_theme(fx.root / "themes")
    protected = build_protected_theme(fx.root / "themes")
    added = _cli(fx, "bucket", "add", "community", str(community))
    runner.check(added.returncode == 0, f"bucket add failed: {added.stderr.strip()}")
    registered = _cli(fx, "theme", "register", str(local))
    runner.check(registered.returncode == 0, f"theme register failed: {registered.stderr.strip()}")

    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    found = _cli(fx, "game", "add", GAME_ID, "--path", game)
    runner.check(found.returncode == 0, f"game add failed: {found.stderr.strip()}")

    _step1_cli(runner, fx, protected)

    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")
    official_height = _step2_default(runner, session)
    bold_height = _step3_variant(runner, session, fx, official_height)
    local_height = _step4_themes(runner, session, fx, bold_height)

    app, session = _step5_refusal(runner, app, fx, session)
    _step6_default(runner, session, fx, official_height, local_height)
    _step7_protected(runner, fx, protected)
    app.close()
