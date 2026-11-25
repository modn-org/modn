pub(super) fn component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn extension(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) if (1..=8).contains(&ext.len()) => format!(".{}", component(ext)),
        _ => String::new(),
    }
}

fn named(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn length_and_extension(ext: &str) -> bool {
    ext.is_empty()
        || ext.strip_prefix('.').is_some_and(|ext| {
            (1..=8).contains(&ext.len())
                && ext
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
}

fn entry_parts(name: &str) -> Option<[&str; 4]> {
    let parts: Vec<&str> = name.split('#').collect();
    let [game_id, mod_id, version, rest] = parts.as_slice() else {
        return None;
    };
    (named(game_id) && named(mod_id) && named(version))
        .then_some([*game_id, *mod_id, *version, *rest])
}

pub(super) fn parse_entry_name(name: &str) -> Option<[&str; 3]> {
    let [game_id, mod_id, version, rest] = entry_parts(name)?;
    let valid_rest = if let Some(git) = rest.strip_prefix("git-") {
        git.split_once('-').is_some_and(|(commit, archive)| {
            hex(commit, 40)
                && archive.strip_prefix("blake3-").is_some_and(|archive| {
                    archive
                        .strip_suffix(".zip")
                        .is_some_and(|digest| hex(digest, 64))
                })
        })
    } else {
        rest.split_at_checked(7)
            .is_some_and(|(url_hash, ext)| hex(url_hash, 7) && length_and_extension(ext))
    };
    valid_rest.then_some([game_id, mod_id, version])
}

pub(super) fn parse_retired_entry_name(name: &str) -> Option<[&str; 3]> {
    let [game_id, mod_id, version, rest] = entry_parts(name)?;
    let retired = rest.strip_prefix("git-").is_some_and(|git| {
        git.split_once('-').is_some_and(|(commit, archive)| {
            hex(commit, 40)
                && archive
                    .strip_suffix(".zip")
                    .is_some_and(|digest| hex(digest, 64))
        })
    });
    retired.then_some([game_id, mod_id, version])
}
