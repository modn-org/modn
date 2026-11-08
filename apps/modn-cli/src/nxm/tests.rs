use super::{parse, parse_all};

const KEY: &str = "NXM_KEY_ABC123";
const EXPIRES: u64 = 1_700_000_100;
const NOW: u64 = 1_700_000_000;

fn valid_link() -> String {
    format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}&user_id=42")
}

fn assert_hidden(error: &anyhow::Error) {
    let text = format!("{error:#}");
    assert!(!text.contains(KEY), "{text}");
    assert!(!text.contains(&EXPIRES.to_string()), "{text}");
}

#[test]
fn parses_a_free_account_link() {
    let link = parse(&valid_link(), NOW).expect("valid nxm link");
    assert_eq!(link.file.0.as_str(), "cyberpunk2077");
    assert_eq!((link.file.1, link.file.2), (107, 123));
    assert_eq!(link.grant.expose_secret(), KEY);
}

#[test]
fn accepts_links_without_a_user_id() {
    let link = format!("nxm://cyberpunk2077/mods/1/files/9999999999?expires={EXPIRES}&key={KEY}");
    let parsed = parse(&link, NOW).expect("valid nxm link");
    assert_eq!((parsed.file.1, parsed.file.2), (1, 9_999_999_999));
}

#[test]
fn debug_redacts_the_key_and_expiry() {
    let link = parse(&valid_link(), NOW).expect("valid nxm link");
    let debug = format!("{link:?}");
    assert!(!debug.contains(KEY), "{debug}");
    assert!(!debug.contains(&EXPIRES.to_string()), "{debug}");
}

#[test]
fn rejects_each_bad_link_form_without_disclosing_values() {
    let now = NOW;
    let collection =
        format!("nxm://cyberpunk2077/collections/107/files/123?key={KEY}&expires={EXPIRES}");
    let query = format!("key={KEY}&expires={EXPIRES}");
    let cases = [
        format!("https://cyberpunk2077/mods/107/files/123?{query}"),
        format!("nxm:/cyberpunk2077/mods/107/files/123?{query}"),
        format!("nxm://Cyberpunk2077/mods/107/files/123?{query}"),
        format!("nxm:///mods/107/files/123?{query}"),
        format!("nxm://user@cyberpunk2077/mods/107/files/123?{query}"),
        format!("nxm://cyberpunk2077:443/mods/107/files/123?{query}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?{query}#fragment"),
        collection,
        format!("nxm://cyberpunk2077/mods/107?{query}"),
        format!("nxm://cyberpunk2077/mods/107/files/123/extra?{query}"),
        format!("nxm://cyberpunk2077/mods/0/files/123?{query}"),
        format!("nxm://cyberpunk2077/mods/10000000000/files/123?{query}"),
        format!("nxm://cyberpunk2077/mods/10x7/files/123?{query}"),
        format!("nxm://cyberpunk2077/mods/107/files/0123x?{query}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}&unknown=1"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&key={KEY}&expires={EXPIRES}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}&expires={EXPIRES}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}&user_id=1&user_id=2"),
        format!("nxm://cyberpunk2077/mods/107/files/123?expires={EXPIRES}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}"),
        "nxm://cyberpunk2077/mods/107/files/123".to_owned(),
        format!("nxm://cyberpunk2077/mods/107/files/123?key=&expires={EXPIRES}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires=soon"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}&user_id=x"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={now}"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires=1"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES} extra"),
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires={EXPIRES}\n"),
    ];
    for link in cases {
        let error = parse(&link, now).expect_err("bad nxm link must fail");
        assert_hidden(&error);
    }
}

#[test]
fn names_the_argument_position_without_contents() {
    let links = vec![valid_link(), "nxm://bad host".to_owned()];
    let error = parse_all(&links, NOW).expect_err("bad link must fail");
    let text = format!("{error:#}");
    assert!(text.contains("invalid --nxm link 2"), "{text}");
    assert!(!text.contains(KEY), "{text}");
}

#[test]
fn names_parsed_ids_in_errors() {
    let link = format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires=1");
    let error = parse(&link, NOW).expect_err("expired link must fail");
    let text = format!("{error:#}");
    assert!(text.contains("nxm link for mod 107 file 123"), "{text}");
}
