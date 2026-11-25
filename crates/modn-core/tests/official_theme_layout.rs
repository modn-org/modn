#[path = "official_theme_layout/checker.rs"]
mod checker;
#[path = "official_theme_layout/parser.rs"]
mod parser;

use checker::EngineChecker;
use modn_core::themes::{
    check_sheet, inspect_theme_dir, theme_digest, DeclFacts, RuleFacts, SheetFacts, StyleFacts,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn theme_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/cyberpunk-2077/night-city")
}

fn read_theme_file(name: &str) -> String {
    fs::read_to_string(theme_root().join(name)).expect("read a staged theme file")
}

fn parse_theme_file(name: &str) -> SheetFacts {
    parser::parse_sheet(&read_theme_file(name)).expect("the staged theme CSS parses")
}

fn style_rules<'a>(rules: &'a [RuleFacts], found: &mut Vec<&'a StyleFacts>) {
    for rule in rules {
        match rule {
            RuleFacts::Style(style) => found.push(style),
            RuleFacts::Media(media) => style_rules(&media.rules, found),
            _ => {}
        }
    }
}

fn simple_hook(selector: &str) -> Option<&str> {
    let name = selector.strip_prefix('.')?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(selector)
}

fn is_protected_selector(selector: &str) -> bool {
    selector.contains("modn-review")
        || selector.contains("modn-approval")
        || selector.contains("data-modn-protected")
}

fn is_layout_property(property: &str) -> bool {
    matches!(
        property,
        "display"
            | "order"
            | "position"
            | "inset"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "width"
            | "height"
            | "overflow"
    ) || property.starts_with("grid")
        || property.starts_with("flex")
        || property.starts_with("gap")
        || property == "row-gap"
        || property == "column-gap"
        || property.starts_with("align-")
        || property.starts_with("justify-")
        || property.starts_with("place-")
        || property.starts_with("inset-")
        || property.starts_with("min-")
        || property.starts_with("max-")
        || property.starts_with("margin")
        || property.starts_with("padding")
}

fn is_structural(declaration: &DeclFacts) -> bool {
    match declaration.property.as_str() {
        "display" => matches!(
            declaration.value.trim().to_ascii_lowercase().as_str(),
            "grid" | "flex"
        ),
        "flex-direction" | "order" => true,
        property => property.starts_with("grid-template-"),
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create a copied directory");
    for entry in fs::read_dir(source).expect("list a theme entry") {
        let entry = entry.expect("read a theme entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("theme entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy a theme file");
        }
    }
}

#[test]
fn staged_theme_passes_inspection() {
    let theme =
        inspect_theme_dir(&theme_root(), &EngineChecker).expect("the staged theme is valid");
    assert_eq!(theme.manifest.id, "night-city");
    assert_eq!(theme.manifest.game, "cyberpunk-2077");
    assert_eq!(theme.manifest.name, "Night City");
    assert_eq!(theme.manifest.theme_api, 1);
    assert_eq!(theme.manifest.variants, vec!["neon", "noir"]);
    assert_eq!(theme.manifest.icon.as_deref(), Some("assets/icon.png"));
    assert!(
        theme.css.contains("@font-face"),
        "canonical base CSS is returned"
    );
    assert_eq!(
        theme.variant_css.keys().cloned().collect::<Vec<_>>(),
        vec!["neon".to_owned(), "noir".to_owned()]
    );
    assert!(!theme.assets.is_empty(), "the theme has assets");
}

#[test]
fn every_stylesheet_passes_the_rule_engine_and_resolves_its_urls() {
    for file in ["theme.css", "theme.neon.css", "theme.noir.css"] {
        let sheet = parse_theme_file(file);
        let outcome = check_sheet(file, &sheet).expect("the stylesheet passes the rule engine");
        assert!(
            !outcome.asset_urls.is_empty() || file != "theme.css",
            "{file} reports its assets"
        );
        for url in outcome.asset_urls {
            assert!(
                theme_root().join(&url).is_file(),
                "{file} references {url}, which exists"
            );
        }
    }
}

#[test]
fn theme_changes_layout_outside_protected_regions() {
    let sheet = parse_theme_file("theme.css");
    let mut rules = Vec::new();
    style_rules(&sheet.rules, &mut rules);
    let mut layout = 0usize;
    let mut structural = 0usize;
    let mut hooks = BTreeSet::new();
    for rule in rules {
        for selector in &rule.selectors {
            let Some(hook) = simple_hook(selector) else {
                continue;
            };
            if is_protected_selector(hook) {
                continue;
            }
            for declaration in &rule.declarations {
                if is_layout_property(&declaration.property) {
                    layout += 1;
                    hooks.insert(hook.to_owned());
                    if is_structural(declaration) {
                        structural += 1;
                    }
                }
            }
        }
    }
    assert!(layout >= 8, "theme.css has {layout} layout declarations");
    assert!(hooks.len() >= 4, "layout touches {} hooks", hooks.len());
    assert!(
        structural >= 2,
        "theme.css has {structural} structural declarations"
    );
}

#[test]
fn protected_rules_only_recolor_and_refont() {
    let sheet = parse_theme_file("theme.css");
    let mut rules = Vec::new();
    style_rules(&sheet.rules, &mut rules);
    let allowed = [
        "color",
        "background-color",
        "border-color",
        "outline-color",
        "font-family",
        "font-weight",
        "font-style",
    ];
    let mut protected_rules = 0usize;
    for rule in rules {
        if !rule
            .selectors
            .iter()
            .any(|selector| is_protected_selector(selector))
        {
            continue;
        }
        protected_rules += 1;
        for declaration in &rule.declarations {
            assert!(
                allowed.contains(&declaration.property.as_str()),
                "protected hook rule declares {}",
                declaration.property
            );
        }
    }
    assert!(protected_rules >= 1, "the theme restyles a protected hook");
    check_sheet("theme.css", &sheet).expect("the rule engine accepts the protected rules");
}

#[test]
fn variants_are_listed_and_differ() {
    let theme =
        inspect_theme_dir(&theme_root(), &EngineChecker).expect("the staged theme is valid");
    let neon = read_theme_file("theme.neon.css");
    let noir = read_theme_file("theme.noir.css");
    assert_ne!(neon, noir, "the two variants differ");
    for (variant, text) in [("neon", &neon), ("noir", &noir)] {
        assert!(
            theme
                .manifest
                .variants
                .iter()
                .any(|listed| listed == variant),
            "{variant} is listed in theme.json"
        );
        let sheet = parser::parse_sheet(text).expect("the variant parses");
        let mut rules = Vec::new();
        style_rules(&sheet.rules, &mut rules);
        let declarations: usize = rules.iter().map(|rule| rule.declarations.len()).sum();
        assert!(
            declarations >= 5,
            "{variant} has {declarations} declarations"
        );
    }
}

#[test]
fn fixed_position_mutation_fails_inspection() {
    let temp = tempfile::tempdir().expect("temp dir");
    let copy = temp.path().join("night-city");
    copy_tree(&theme_root(), &copy);
    let css = fs::read_to_string(copy.join("theme.css")).expect("read the copied CSS");
    let newline = if css.contains("\r\n") { "\r\n" } else { "\n" };
    let mutated = css.replacen(
        ".modn-rail {",
        &format!(".modn-rail {{{newline}position: fixed;"),
        1,
    );
    assert_ne!(mutated, css, "the mutation inserted a declaration");
    fs::write(copy.join("theme.css"), mutated).expect("write the mutated CSS");
    let problems = inspect_theme_dir(&copy, &EngineChecker).expect_err("the mutation is refused");
    assert!(
        problems
            .iter()
            .any(|problem| problem.rule.id() == "forbidden-position-fixed"),
        "the real engine reports forbidden-position-fixed"
    );
}

#[test]
fn digest_is_stable_lowercase_hex() {
    let first = theme_digest(&theme_root()).expect("digest the staged theme");
    let second = theme_digest(&theme_root()).expect("digest the staged theme again");
    assert_eq!(first, second, "the digest is identical across two calls");
    assert_eq!(first.len(), 64, "the digest is 64 hex characters");
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "the digest is lowercase hex"
    );
}

#[test]
fn assets_and_entries_stay_within_the_limits() {
    let allowed: BTreeSet<&str> = [
        "theme.json",
        "theme.css",
        "theme.neon.css",
        "theme.noir.css",
        "assets",
        "LICENSE",
        "NOTICE.md",
    ]
    .into_iter()
    .collect();
    let root = theme_root();
    let mut asset_total = 0u64;
    let mut assets = 0usize;
    for entry in fs::read_dir(&root).expect("list the theme folder") {
        let entry = entry.expect("read a theme entry");
        let name = entry.file_name();
        assert!(
            allowed.contains(name.to_str().expect("UTF-8 entry")),
            "top-level entry {name:?} is allowed"
        );
        if !entry.file_type().expect("entry type").is_dir() {
            continue;
        }
        assert_eq!(name, "assets", "the only subdirectory is assets");
        for asset in fs::read_dir(entry.path()).expect("list the assets folder") {
            let asset = asset.expect("read an asset");
            assert!(asset.file_type().expect("asset type").is_file());
            asset_total += asset.metadata().expect("asset metadata").len();
            assets += 1;
        }
    }
    assert!(assets <= 64, "the theme has {assets} assets");
    assert!(
        asset_total < 1024 * 1024,
        "assets total {asset_total} bytes"
    );
}

#[test]
fn font_and_tile_are_present_and_wired_in() {
    let css = read_theme_file("theme.css");
    assert!(
        css.contains("src: url(assets/chakra-petch-semibold.woff2);"),
        "theme.css loads the bundled font"
    );
    assert!(
        css.contains("font-family: \"Chakra Petch\""),
        "theme.css uses the bundled font family"
    );
    assert!(
        css.contains("background-image: url(assets/grid.png);"),
        "theme.css uses the generated tile"
    );
}
