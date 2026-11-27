//! A small `*` and `?` glob matcher for upstream asset and file names.

/// Match `name` against `pattern`.
///
/// `*` matches any sequence of zero or more characters and `?` matches
/// exactly one character. Every other character matches itself. Matching is
/// case-sensitive. A pattern without a wildcard is an exact comparison.
#[must_use]
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_name = 0usize;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            star_name = n;
            p += 1;
        } else if let Some(index) = star {
            p = index + 1;
            star_name += 1;
            n = star_name;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    #[test]
    fn checkver_glob_matches_exact_and_wildcard_names() {
        assert!(glob_matches("Mod-*.zip", "Mod-1.2.zip"));
        assert!(glob_matches("Mod-*.zip", "Mod-.zip"));
        assert!(glob_matches("Mod-?.zip", "Mod-1.zip"));
        assert!(!glob_matches("Mod-?.zip", "Mod-12.zip"));
        assert!(glob_matches("*-*.zip", "any-name.zip"));
        assert!(glob_matches("*.zip", "a.zip"));
        assert!(!glob_matches("*.zip", "a.7z"));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(!glob_matches("a*b*c", "aXXbYY"));
        assert!(glob_matches("exact.zip", "exact.zip"));
        assert!(!glob_matches("exact.zip", "Exact.zip"));
        assert!(!glob_matches("exact.zip", "exact.zip.bak"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("?", "x"));
        assert!(!glob_matches("?", ""));
    }
}
