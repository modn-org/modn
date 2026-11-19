use super::{Rule, ThemeManifest, ThemeProblem};
use serde_json::{json, Value};

fn valid() -> Value {
    json!({
        "schema_version": 1,
        "id": "night-city",
        "game": "cyberpunk-2077",
        "name": "Night City",
        "theme_api": 1,
        "variants": ["neon", "noir"],
        "icon": "assets/icon.png"
    })
}

fn parse(value: &Value) -> Result<ThemeManifest, ThemeProblem> {
    ThemeManifest::parse(value.to_string().as_bytes())
}

fn rejected(value: &Value) -> ThemeProblem {
    parse(value).expect_err("manifest should be rejected")
}

#[test]
fn a_complete_manifest_parses() {
    let manifest = parse(&valid()).unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.id, "night-city");
    assert_eq!(manifest.game, "cyberpunk-2077");
    assert_eq!(manifest.name, "Night City");
    assert_eq!(manifest.theme_api, 1);
    assert_eq!(manifest.variants, ["neon", "noir"]);
    assert_eq!(manifest.icon.as_deref(), Some("assets/icon.png"));
}

#[test]
fn a_manifest_without_icon_or_variants_parses() {
    let mut value = valid();
    value.as_object_mut().unwrap().remove("icon");
    value["variants"] = json!([]);
    let manifest = parse(&value).unwrap();
    assert_eq!(manifest.icon, None);
    assert!(manifest.variants.is_empty());
}

#[test]
fn a_parsed_manifest_serialises_back_to_the_same_fields() {
    let manifest = parse(&valid()).unwrap();
    let value = serde_json::to_value(&manifest).unwrap();
    assert_eq!(value["id"], "night-city");
    assert_eq!(value["game"], "cyberpunk-2077");
    assert_eq!(value["name"], "Night City");
    assert_eq!(value["theme_api"], 1);
    assert_eq!(value["variants"], json!(["neon", "noir"]));
    assert_eq!(value["icon"], "assets/icon.png");
}

#[test]
fn an_unknown_field_is_named_in_its_message() {
    let mut value = valid();
    value["extra"] = json!(true);
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::UnknownManifestField);
    assert_eq!(problem.path.as_deref(), Some("theme.json"));
    assert!(problem.message.contains("extra"), "{}", problem.message);
}

#[test]
fn a_wrong_schema_version_is_manifest_invalid() {
    let mut value = valid();
    value["schema_version"] = json!(2);
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("schema_version"));
}

#[test]
fn a_wrong_theme_api_is_manifest_invalid() {
    let mut value = valid();
    value["theme_api"] = json!(2);
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("theme_api"));
}

#[test]
fn an_invalid_theme_id_is_manifest_invalid() {
    for id in ["Night-City", "night_city", "", "night city"] {
        let mut value = valid();
        value["id"] = json!(id);
        let problem = rejected(&value);
        assert_eq!(problem.rule, Rule::ManifestInvalid, "id {id:?}");
    }
}

#[test]
fn an_invalid_game_id_is_manifest_invalid() {
    let mut value = valid();
    value["game"] = json!("Cyberpunk");
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("game id"));
}

#[test]
fn an_invalid_variant_id_is_manifest_invalid() {
    let mut value = valid();
    value["variants"] = json!(["neon", "Noir"]);
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("variant id"));
}

#[test]
fn a_duplicate_variant_is_manifest_invalid() {
    let mut value = valid();
    value["variants"] = json!(["neon", "neon"]);
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("more than once"));
}

#[test]
fn sixteen_variants_are_accepted_and_seventeen_are_rejected() {
    let variants =
        |count: usize| -> Vec<String> { (0..count).map(|index| format!("v{index}")).collect() };
    let mut value = valid();
    value["variants"] = json!(variants(16));
    assert_eq!(parse(&value).unwrap().variants.len(), 16);
    value["variants"] = json!(variants(17));
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("16"));
}

#[test]
fn a_name_of_one_to_64_characters_is_required() {
    let mut value = valid();
    for name in [String::new(), "x".repeat(65)] {
        value["name"] = json!(name);
        let problem = rejected(&value);
        assert_eq!(
            problem.rule,
            Rule::ManifestInvalid,
            "name length {}",
            name.len()
        );
    }
    value["name"] = json!("x".repeat(64));
    assert_eq!(parse(&value).unwrap().name.chars().count(), 64);
}

#[test]
fn a_name_with_a_control_character_is_rejected() {
    let mut value = valid();
    value["name"] = json!("Night\u{7}City");
    let problem = rejected(&value);
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("control"));
}

#[test]
fn an_icon_must_be_a_png_or_webp_path_under_assets() {
    for icon in [
        "assets/icon.gif",
        "icons/icon.png",
        "assets/Icon.png",
        "assets/../icon.png",
    ] {
        let mut value = valid();
        value["icon"] = json!(icon);
        let problem = rejected(&value);
        assert_eq!(problem.rule, Rule::ManifestInvalid, "icon {icon}");
    }
    let mut value = valid();
    value["icon"] = json!("assets/icon.webp");
    assert_eq!(parse(&value).unwrap().icon.unwrap(), "assets/icon.webp");
}

#[test]
fn malformed_and_incomplete_json_is_manifest_invalid() {
    for bytes in [
        b"{".as_slice(),
        b"null".as_slice(),
        b"[]".as_slice(),
        br#"{"schema_version":1}"#.as_slice(),
    ] {
        let problem = ThemeManifest::parse(bytes).expect_err("invalid manifest");
        assert_eq!(problem.rule, Rule::ManifestInvalid);
        assert_eq!(problem.path.as_deref(), Some("theme.json"));
    }
}

#[test]
fn a_wrongly_typed_field_is_manifest_invalid() {
    let mut value = valid();
    value["schema_version"] = json!("1");
    assert_eq!(rejected(&value).rule, Rule::ManifestInvalid);
}

#[test]
fn a_manifest_over_16_kib_is_manifest_invalid() {
    let bytes = vec![b' '; super::super::limits::MAX_THEME_MANIFEST_BYTES as usize + 1];
    let problem = ThemeManifest::parse(&bytes).expect_err("oversized manifest");
    assert_eq!(problem.rule, Rule::ManifestInvalid);
    assert!(problem.message.contains("16384"), "{}", problem.message);
}
