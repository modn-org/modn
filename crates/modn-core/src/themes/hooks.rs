//! Stable theme hooks generated from `docs/themes.md`.
//!
//! The tables in the "Stable hooks" section are the source of truth. The
//! tests in `hooks_tests.rs` compare these lists with the document, so a hook
//! change must update both.

/// Stable `.modn-*` class names, without the leading `.`.
pub const STABLE_CLASSES: &[&str] = &[
    "modn-actions",
    "modn-app",
    "modn-approval",
    "modn-approval__actions",
    "modn-approval__button",
    "modn-approval__button--quiet",
    "modn-approval__list",
    "modn-approval__path",
    "modn-approval__question",
    "modn-button",
    "modn-button--danger",
    "modn-button--quiet",
    "modn-button--secondary",
    "modn-card",
    "modn-card--empty",
    "modn-details",
    "modn-details__actions",
    "modn-details__ask",
    "modn-details__back",
    "modn-details__description",
    "modn-details__footer",
    "modn-details__header",
    "modn-details__heading",
    "modn-details__meta",
    "modn-details__state",
    "modn-details__title",
    "modn-details__titles",
    "modn-empty",
    "modn-game-card",
    "modn-game-card__badge",
    "modn-game-card__body",
    "modn-game-card__error",
    "modn-game-card__hint",
    "modn-game-card__initials",
    "modn-game-card__media",
    "modn-game-card__meta",
    "modn-game-card__path",
    "modn-game-card__remove",
    "modn-game-card__surface",
    "modn-game-card__title",
    "modn-grid",
    "modn-history",
    "modn-history__entry",
    "modn-history__label",
    "modn-home__actions",
    "modn-home__empty",
    "modn-home__grid",
    "modn-input",
    "modn-library",
    "modn-library__actions",
    "modn-library__ask",
    "modn-library__empty",
    "modn-library__grid",
    "modn-library__hero",
    "modn-library__installation",
    "modn-library__label",
    "modn-library__search",
    "modn-library__title",
    "modn-library__toolbar",
    "modn-link",
    "modn-list",
    "modn-main",
    "modn-menu",
    "modn-menu__item",
    "modn-muted",
    "modn-op",
    "modn-op__bar",
    "modn-op__button",
    "modn-op__progress",
    "modn-rail",
    "modn-rail__link",
    "modn-recent",
    "modn-recent__entry",
    "modn-recent__menu",
    "modn-recent__text",
    "modn-review",
    "modn-review__approval",
    "modn-review__blocker",
    "modn-review__body",
    "modn-review__list",
    "modn-review__muted",
    "modn-review__path",
    "modn-review__row",
    "modn-review__status",
    "modn-review__status--error",
    "modn-review__summary",
    "modn-review__tag",
    "modn-row",
    "modn-row__text",
    "modn-select",
    "modn-status",
    "modn-status--error",
    "modn-tab",
    "modn-tab--active",
    "modn-tabs",
    "modn-tag",
    "modn-theme-actions",
    "modn-theme-button",
    "modn-theme-control",
    "modn-theme-current",
    "modn-theme-diagnostic",
    "modn-theme-diagnostics",
    "modn-theme-item",
    "modn-theme-list",
    "modn-theme-mark",
    "modn-theme-name",
    "modn-theme-panel",
    "modn-theme-pick",
    "modn-theme-pick--chosen",
    "modn-theme-select",
    "modn-theme-shell",
    "modn-theme-source",
    "modn-theme-status",
    "modn-theme-variant",
    "modn-tile",
    "modn-tile__actions",
    "modn-tile__art",
    "modn-tile__ask",
    "modn-tile__body",
    "modn-tile__initials",
    "modn-tile__name",
    "modn-tile__state",
    "modn-tile__tags",
    "modn-tile__warning",
    "modn-toast",
    "modn-toast__action",
    "modn-toast__message",
    "modn-toasts",
    "modn-view",
    "modn-view__header",
];

/// Stable `data-*` attribute names, without the brackets.
pub const STABLE_DATA_ATTRIBUTES: &[&str] = &[
    "data-busy",
    "data-code",
    "data-game-id",
    "data-installed",
    "data-mod-id",
    "data-modn-protected",
    "data-operation",
    "data-operation-id",
    "data-path",
    "data-state",
    "data-testid",
    "data-theme",
];

/// Stable `--modn-*` custom property names.
pub const STABLE_CUSTOM_PROPERTIES: &[&str] = &[
    "--modn-color-border",
    "--modn-color-danger",
    "--modn-color-on-primary",
    "--modn-color-primary",
    "--modn-color-primary-hover",
    "--modn-color-success",
    "--modn-color-surface",
    "--modn-color-surface-raised",
    "--modn-color-text",
    "--modn-color-text-muted",
    "--modn-font-body",
    "--modn-font-heading",
    "--modn-font-line-body-l",
    "--modn-font-line-body-m",
    "--modn-font-line-body-s",
    "--modn-font-line-display",
    "--modn-font-line-heading-l",
    "--modn-font-line-heading-m",
    "--modn-font-line-heading-s",
    "--modn-font-line-label-m",
    "--modn-font-line-label-s",
    "--modn-font-size-body-l",
    "--modn-font-size-body-m",
    "--modn-font-size-body-s",
    "--modn-font-size-display",
    "--modn-font-size-heading-l",
    "--modn-font-size-heading-m",
    "--modn-font-size-heading-s",
    "--modn-font-size-label-m",
    "--modn-font-size-label-s",
    "--modn-opacity-disabled",
    "--modn-radius-l",
    "--modn-radius-m",
    "--modn-radius-s",
    "--modn-space-2xl",
    "--modn-space-2xs",
    "--modn-space-3xl",
    "--modn-space-4xl",
    "--modn-space-l",
    "--modn-space-m",
    "--modn-space-s",
    "--modn-space-xl",
    "--modn-space-xs",
];

/// Protected region class prefixes, without the leading `.`.
pub const PROTECTED_CLASS_PREFIXES: &[&str] = &["modn-review", "modn-approval"];

/// The attribute that marks a protected region.
pub const PROTECTED_REGION_ATTRIBUTE: &str = "data-modn-protected";

/// Return whether `name` is a stable class, without the leading `.`.
#[must_use]
pub fn is_stable_class(name: &str) -> bool {
    STABLE_CLASSES.contains(&name)
}

/// Return whether `name` is a stable data attribute, without brackets.
#[must_use]
pub fn is_stable_data_attribute(name: &str) -> bool {
    STABLE_DATA_ATTRIBUTES.contains(&name)
}

/// Return whether `name` is a protected class, without the leading `.`.
#[must_use]
pub fn is_protected_class(name: &str) -> bool {
    PROTECTED_CLASS_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
