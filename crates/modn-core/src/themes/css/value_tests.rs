use super::{
    colour_is_opaque, find_url_targets, forbidden_image_function, is_remote_url,
    normalise_asset_target,
};

#[test]
fn url_targets_are_found_with_every_quote_style() {
    let value = "url(assets/a.png), URL( \"assets/b.webp\" ), url('assets/c.woff2')";
    assert_eq!(
        find_url_targets(value),
        ["assets/a.png", "assets/b.webp", "assets/c.woff2"]
    );
}

#[test]
fn url_targets_keep_inner_spaces_and_reject_malformed_tokens() {
    assert_eq!(find_url_targets("url( assets/a.png )"), ["assets/a.png"]);
    assert!(find_url_targets("url(").is_empty());
    assert!(find_url_targets("url(\"assets/a.png)").is_empty());
    assert_eq!(find_url_targets("url()"), [""]);
    assert!(find_url_targets("no url here").is_empty());
}

#[test]
fn remote_targets_are_recognised() {
    for target in [
        "https://example.com/a.png",
        "HTTP://example.com/a.png",
        "//example.com/a.png",
        "/assets/a.png",
        "data:image/png;base64,AAAA",
        "javascript:alert(1)",
        "C:/theme/assets/a.png",
    ] {
        assert!(is_remote_url(target), "{target} should be remote");
    }
    for target in ["assets/a.png", "./assets/a.png", "assets/a:b.png"] {
        assert!(!is_remote_url(target), "{target} should be local");
    }
}

#[test]
fn asset_targets_are_normalised() {
    assert_eq!(
        normalise_asset_target("assets/a.png").as_deref(),
        Some("assets/a.png")
    );
    assert_eq!(
        normalise_asset_target("./assets/a.webp").as_deref(),
        Some("assets/a.webp")
    );
    assert_eq!(
        normalise_asset_target("././assets/a.woff2").as_deref(),
        Some("assets/a.woff2")
    );
    assert_eq!(
        normalise_asset_target("assets/sub/a.PNG").as_deref(),
        Some("assets/sub/a.PNG")
    );
}

#[test]
fn targets_outside_assets_or_with_other_extensions_are_rejected() {
    for target in [
        "../assets/a.png",
        "assets/../a.png",
        "assets/./a.png",
        "other/a.png",
        "assets/a.jpg",
        "assets/a",
        "assets/",
        "assets/.png",
        "assets/a.png?x=1",
        "assets//a.png",
        "./a.png",
        "assets\\a.png",
    ] {
        assert!(
            normalise_asset_target(target).is_none(),
            "{target} should be rejected"
        );
    }
}

#[test]
fn image_loading_functions_are_detected() {
    for value in [
        "image-set(assets/a.png)",
        "IMAGE-SET(url(assets/a.png))",
        "-webkit-image-set(assets/a.png)",
        "cross-fade(assets/a.png, assets/b.png)",
        "image(assets/a.png)",
        "src(assets/a.png)",
        "element(#modn-card)",
    ] {
        assert!(
            forbidden_image_function(value).is_some(),
            "{value} should be rejected"
        );
    }
    assert!(forbidden_image_function("url(assets/a.png)").is_none());
    assert!(forbidden_image_function("rgb(0, 0, 0)").is_none());
}

#[test]
fn opaque_colours_are_accepted() {
    for value in [
        "#fff",
        "#ffffff",
        "#ffff",
        "#ffffffff",
        "#FFFFFF",
        "rgb(0, 0, 0)",
        "rgba(0, 0, 0)",
        "rgba(0, 0, 0, 1)",
        "rgb(0 0 0 / 1)",
        "rgb(0 0 0/100%)",
        "hsl(0, 0%, 0%)",
        "hsla(0 0% 0% / 100%)",
        "black",
        "RebeccaPurple",
        "white",
    ] {
        assert!(colour_is_opaque(value), "{value} should be opaque");
    }
}

#[test]
fn translucent_or_unknown_colours_are_rejected() {
    for value in [
        "#fff0",
        "#ffffff00",
        "#fff8",
        "rgb(0 0 0 / 0.5)",
        "rgb(0 0 0/0.5)",
        "rgba(0, 0, 0, 0.5)",
        "hsl(0 0% 0% / 50%)",
        "transparent",
        "currentcolor",
        "var(--modn-color-text)",
        "inherit",
        "color-mix(in srgb, red, blue)",
        "rgb()",
        "rgb(0)",
        "rgb(0 0 var(--modn-color-text) / 1)",
        "notacolour",
    ] {
        assert!(!colour_is_opaque(value), "{value} should be rejected");
    }
}
