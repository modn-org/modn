use super::{
    is_protected_class, is_protected_context_class, is_protected_reachable_attribute,
    is_stable_class, is_stable_data_attribute, PROTECTED_CLASS_PREFIXES, PROTECTED_CONTEXT_CLASSES,
    PROTECTED_REACHABLE_ATTRIBUTES, PROTECTED_REGION_ATTRIBUTE, STABLE_CLASSES,
    STABLE_CUSTOM_PROPERTIES, STABLE_DATA_ATTRIBUTES,
};

const DOC: &str = include_str!("../../../../docs/themes.md");

fn section(heading: &str) -> &'static str {
    let start = DOC
        .find(heading)
        .unwrap_or_else(|| panic!("docs/themes.md has no heading \"{heading}\""))
        + heading.len();
    let rest = &DOC[start..];
    let end = rest
        .find("\n### ")
        .into_iter()
        .chain(rest.find("\n## "))
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

fn table_names(heading: &str) -> Vec<String> {
    section(heading)
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("| `")?;
            let end = rest.find('`')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

fn assert_same(doc: &[String], code: &[&str], const_name: &str) {
    for hook in doc {
        if !code.contains(&hook.as_str()) {
            panic!("hook `{hook}` is in docs/themes.md but missing from {const_name}");
        }
    }
    for hook in code {
        if !doc.iter().any(|documented| documented == hook) {
            panic!("hook `{hook}` is in {const_name} but missing from docs/themes.md");
        }
    }
}

#[test]
fn stable_classes_match_the_document() {
    let doc: Vec<String> = table_names("### Classes")
        .into_iter()
        .map(|name| name.trim_start_matches('.').to_string())
        .collect();
    assert_eq!(doc.len(), 130, "classes table should hold 130 rows");
    assert_same(&doc, STABLE_CLASSES, "STABLE_CLASSES");
}

#[test]
fn stable_data_attributes_match_the_document() {
    let doc: Vec<String> = table_names("### Data attributes")
        .into_iter()
        .map(|name| name.trim_matches(['[', ']']).to_string())
        .collect();
    assert_eq!(doc.len(), 12, "data attribute table should hold 12 rows");
    assert_same(&doc, STABLE_DATA_ATTRIBUTES, "STABLE_DATA_ATTRIBUTES");
}

#[test]
fn stable_custom_properties_match_the_document() {
    let doc = table_names("### Custom properties");
    assert_eq!(doc.len(), 43, "custom property table should hold 43 rows");
    assert_same(&doc, STABLE_CUSTOM_PROPERTIES, "STABLE_CUSTOM_PROPERTIES");
}

#[test]
fn protected_context_classes_are_stable_and_documented() {
    for name in PROTECTED_CONTEXT_CLASSES {
        assert!(
            is_stable_class(name),
            "context class \"{name}\" must be a stable class"
        );
    }
    let doc: Vec<String> = table_names("### Context hooks")
        .into_iter()
        .map(|name| name.trim_start_matches('.').to_string())
        .collect();
    assert_eq!(
        doc.len(),
        PROTECTED_CONTEXT_CLASSES.len(),
        "context hook table should hold {} rows",
        PROTECTED_CONTEXT_CLASSES.len()
    );
    assert_same(&doc, PROTECTED_CONTEXT_CLASSES, "PROTECTED_CONTEXT_CLASSES");
}

#[test]
fn protected_reachable_attributes_are_stable_and_documented() {
    for name in PROTECTED_REACHABLE_ATTRIBUTES {
        assert!(
            is_stable_data_attribute(name),
            "reachable attribute \"{name}\" must be a stable data attribute"
        );
    }
    let doc: Vec<String> = table_names("### Reachable attributes")
        .into_iter()
        .map(|name| name.trim_matches(['[', ']']).to_string())
        .collect();
    assert_eq!(
        doc.len(),
        PROTECTED_REACHABLE_ATTRIBUTES.len(),
        "reachable attribute table should hold {} rows",
        PROTECTED_REACHABLE_ATTRIBUTES.len()
    );
    assert_same(
        &doc,
        PROTECTED_REACHABLE_ATTRIBUTES,
        "PROTECTED_REACHABLE_ATTRIBUTES",
    );
}

#[test]
fn the_generated_lists_are_sorted_and_unique() {
    for (name, list) in [
        ("STABLE_CLASSES", STABLE_CLASSES),
        ("STABLE_DATA_ATTRIBUTES", STABLE_DATA_ATTRIBUTES),
        ("STABLE_CUSTOM_PROPERTIES", STABLE_CUSTOM_PROPERTIES),
        ("PROTECTED_CONTEXT_CLASSES", PROTECTED_CONTEXT_CLASSES),
        (
            "PROTECTED_REACHABLE_ATTRIBUTES",
            PROTECTED_REACHABLE_ATTRIBUTES,
        ),
    ] {
        let mut sorted = list.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(list, sorted.as_slice(), "{name} must be sorted and unique");
    }
}

#[test]
fn helpers_recognise_only_listed_hooks() {
    assert!(is_stable_class("modn-card"));
    assert!(!is_stable_class("modn-missing"));
    assert!(!is_stable_class(".modn-card"));
    assert!(is_stable_data_attribute("data-theme"));
    assert!(!is_stable_data_attribute("[data-theme]"));
    assert!(!is_stable_data_attribute("data-missing"));
    assert!(is_protected_class("modn-review__status"));
    assert!(is_protected_class("modn-approval__button"));
    assert!(!is_protected_class("modn-card"));
    assert_eq!(PROTECTED_CLASS_PREFIXES, &["modn-review", "modn-approval"]);
    assert_eq!(PROTECTED_REGION_ATTRIBUTE, "data-modn-protected");
    assert!(is_protected_reachable_attribute("data-testid"));
    assert!(!is_protected_reachable_attribute("data-theme"));
    assert!(is_protected_context_class("modn-app"));
    assert!(!is_protected_context_class("modn-card"));
}
