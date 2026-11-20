use super::{
    is_valid_asset_path, is_valid_id, MAX_THEME_ASSETS, MAX_THEME_ASSETS_BYTES,
    MAX_THEME_ASSET_BYTES, MAX_THEME_CSS_BYTES, MAX_THEME_MANIFEST_BYTES, MAX_THEME_VARIANTS,
    MAX_THEME_VARIANT_CSS_BYTES,
};

#[test]
fn valid_ids_are_accepted() {
    for id in ["a", "0", "night-city", "cyberpunk-2077", "a1-b2-c3", "x9"] {
        assert!(is_valid_id(id), "{id} should be valid");
    }
}

#[test]
fn the_id_length_boundary_is_64_characters() {
    let at_limit = "a".repeat(64);
    let over_limit = "a".repeat(65);
    assert!(is_valid_id(&at_limit));
    assert!(!is_valid_id(&over_limit));
}

#[test]
fn invalid_ids_are_rejected() {
    for id in [
        "",
        "-",
        "-leading",
        "trailing-",
        "double--hyphen",
        "Uppercase",
        "under_score",
        "with space",
        "dot.dot",
        "slash/id",
        "é",
        "a-",
    ] {
        assert!(!is_valid_id(id), "{id:?} should be invalid");
    }
}

#[test]
fn valid_asset_paths_are_accepted() {
    for path in [
        "assets/a.png",
        "assets/icon.webp",
        "assets/font.woff2",
        "assets/sub/dir/image-1.png",
        "assets/a_b.c-d.png",
        "assets/1.png",
    ] {
        assert!(is_valid_asset_path(path), "{path} should be valid");
    }
}

#[test]
fn asset_paths_need_the_assets_prefix_and_lowercase_names() {
    for path in [
        "assets",
        "assets/",
        "assets//a.png",
        "assets/../a.png",
        "assets/a/../b.png",
        "assets/.hidden.png",
        "assets/A.png",
        "assets/a b.png",
        "assets/a..b.png",
        "other/a.png",
        "assets/foo\\bar.png",
        "/assets/a.png",
        "assets/é.png",
    ] {
        assert!(!is_valid_asset_path(path), "{path} should be invalid");
    }
}

#[test]
fn the_documented_limits_match_the_specification() {
    assert_eq!(MAX_THEME_CSS_BYTES, 262_144);
    assert_eq!(MAX_THEME_VARIANT_CSS_BYTES, 131_072);
    assert_eq!(MAX_THEME_ASSET_BYTES, 2_097_152);
    assert_eq!(MAX_THEME_ASSETS_BYTES, 8_388_608);
    assert_eq!(MAX_THEME_ASSETS, 64);
    assert_eq!(MAX_THEME_VARIANTS, 16);
    assert_eq!(MAX_THEME_MANIFEST_BYTES, 16_384);
}

#[test]
fn an_asset_segment_may_contain_a_single_dot() {
    assert!(is_valid_asset_path("assets/a.b.c.png"));
    assert!(!is_valid_asset_path("assets/a..png"));
}
