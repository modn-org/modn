"""UI Automation activation helpers for the `themes` journey.

The Theme control publishes no single action pattern. The trigger is a button
with `aria-expanded`, so Chromium exposes ExpandCollapse and no Invoke. A theme
pick is a button with `aria-pressed`, so it exposes Toggle. `Session.activate`
tries Invoke first, then only LegacyIAccessible, whose default action does
nothing for either element. These helpers press the pattern the element
publishes.
"""


def _actions(control):
    """Yields each published pattern with the method that presses it."""
    for name, method in (
        ("ExpandCollapse", "Expand"),
        ("Toggle", "Toggle"),
        ("Invoke", "Invoke"),
    ):
        try:
            pattern = getattr(control, f"Get{name}Pattern")()
        except Exception:
            continue
        if pattern is not None:
            yield pattern, method


def press(control):
    """Activates `control` with the first pattern it publishes.

    Falls back to the legacy default action only when no pattern is published.
    """
    for pattern, method in _actions(control):
        try:
            getattr(pattern, method)()
            return
        except Exception:
            continue
    control.GetLegacyIAccessiblePattern().DoDefaultAction()
