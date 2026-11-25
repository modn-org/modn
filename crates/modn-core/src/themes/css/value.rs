const FORBIDDEN_FUNCTIONS: &[&str] = &[
    "image-set(",
    "-webkit-image-set(",
    "cross-fade(",
    "image(",
    "src(",
    "element(",
];

const NAMED_COLOURS: &[&str] = &[
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "rebeccapurple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

#[must_use]
pub(crate) fn find_url_targets(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut targets = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if bytes[index..index + 4].eq_ignore_ascii_case(b"url(") {
            if let Some((target, next)) = parse_url_target(value, index + 4) {
                targets.push(target);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    targets
}

#[must_use]
pub(crate) fn forbidden_image_function(value: &str) -> Option<&'static str> {
    let lower = value.to_ascii_lowercase();
    FORBIDDEN_FUNCTIONS
        .iter()
        .copied()
        .find(|function| lower.contains(function))
}

#[must_use]
pub(crate) fn is_remote_url(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.starts_with("data:")
        || target.starts_with("//")
        || target.starts_with('/')
        || has_scheme(target)
}

#[must_use]
pub(crate) fn normalise_asset_target(target: &str) -> Option<String> {
    if target.contains('\\') {
        return None;
    }
    let mut path = target;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    let mut segments = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        segments.push(segment);
    }
    if segments.len() < 2 || segments[0] != "assets" {
        return None;
    }
    let file = segments.last()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "webp" | "woff2"
    ) {
        return None;
    }
    Some(segments.join("/"))
}

#[must_use]
pub(crate) fn colour_is_opaque(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        return hex_is_opaque(hex);
    }
    if let Some((name, arguments)) = split_function(&value) {
        return matches!(name, "rgb" | "rgba" | "hsl" | "hsla") && arguments_are_opaque(arguments);
    }
    NAMED_COLOURS.contains(&value.as_str())
}

fn parse_url_target(value: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = value.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index < bytes.len() && (bytes[index] == b'\'' || bytes[index] == b'"') {
        let quote = bytes[index];
        let content_start = index + 1;
        let mut end = content_start;
        while end < bytes.len() && bytes[end] != quote {
            end += 1;
        }
        if end >= bytes.len() {
            return None;
        }
        let target = value[content_start..end].trim();
        index = end + 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b')' {
            return Some((target, index + 1));
        }
        return None;
    }
    let target_start = index;
    while index < bytes.len() && bytes[index] != b')' {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }
    Some((value[target_start..index].trim(), index + 1))
}

fn has_scheme(target: &str) -> bool {
    let bytes = target.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for byte in &bytes[1..] {
        if *byte == b':' {
            return true;
        }
        if !(byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'-' | b'.')) {
            return false;
        }
    }
    false
}

fn hex_is_opaque(hex: &str) -> bool {
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    match hex.len() {
        3 | 6 => true,
        4 => hex.ends_with('f'),
        8 => hex.ends_with("ff"),
        _ => false,
    }
}

fn split_function(value: &str) -> Option<(&str, &str)> {
    let open = value.find('(')?;
    if !value.ends_with(')') {
        return None;
    }
    let name = value[..open].trim();
    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    Some((name, &value[open + 1..value.len() - 1]))
}

fn arguments_are_opaque(arguments: &str) -> bool {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return false;
    }
    let (colour, slash_alpha) = match arguments.split_once('/') {
        Some((left, right)) if !right.contains('/') => (left.trim(), Some(right.trim())),
        Some(_) => return false,
        None => (arguments, None),
    };
    if colour.to_ascii_lowercase().contains("var(") {
        return false;
    }
    let parts: Vec<&str> = if colour.contains(',') {
        colour.split(',').map(str::trim).collect()
    } else {
        colour.split_whitespace().collect()
    };
    let alpha = if let Some(alpha) = slash_alpha {
        if parts.len() != 3 {
            return false;
        }
        Some(alpha)
    } else {
        match parts.len() {
            3 => None,
            4 => Some(parts[3]),
            _ => return false,
        }
    };
    alpha.is_none_or(alpha_is_one)
}

fn alpha_is_one(alpha: &str) -> bool {
    let alpha = alpha.trim();
    if let Some(percent) = alpha.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f64>()
            .is_ok_and(|value| value == 100.0);
    }
    alpha.parse::<f64>().is_ok_and(|value| value == 1.0)
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
