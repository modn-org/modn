use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) const ASSET_BASE: &str = "http://modn-theme.localhost/";

const ASSETS_PREFIX: &str = "/assets/";

pub(crate) const ASSET_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ThemeAsset {
    pub path: PathBuf,
    pub content_type: &'static str,
}

#[derive(Debug)]
pub(crate) struct AssetError;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AssetResponse {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub body: Vec<u8>,
}

impl AssetResponse {
    fn status(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
        }
    }
}

pub(crate) fn resolve_asset(root: &Path, request_path: &str) -> Result<ThemeAsset, AssetError> {
    let decoded = decode_assets_path(request_path)?;
    let relative = &decoded[ASSETS_PREFIX.len()..];
    let mut segments = Vec::new();
    for segment in relative.split('/') {
        if !valid_segment(segment) {
            return Err(AssetError);
        }
        segments.push(segment);
    }
    let file_name = segments.last().ok_or(AssetError)?;
    let content_type = content_type(file_name)?;

    let assets = root.join("assets");
    refuse_symlinks(&assets, &segments)?;
    let asset = segments
        .iter()
        .fold(assets.clone(), |path, segment| path.join(segment));
    let canonical = fs::canonicalize(&asset).map_err(|_| AssetError)?;
    let canonical_assets = fs::canonicalize(&assets).map_err(|_| AssetError)?;
    if !canonical.starts_with(&canonical_assets) {
        return Err(AssetError);
    }
    let metadata = fs::metadata(&canonical).map_err(|_| AssetError)?;
    if !metadata.is_file() || metadata.len() > ASSET_LIMIT {
        return Err(AssetError);
    }
    Ok(ThemeAsset {
        path: canonical,
        content_type,
    })
}

pub(crate) fn load_asset(root: Option<&Path>, method: &str, request_path: &str) -> AssetResponse {
    if method != "GET" {
        return AssetResponse::status(405);
    }
    let Some(root) = root else {
        return AssetResponse::status(404);
    };
    let Ok(asset) = resolve_asset(root, request_path) else {
        return AssetResponse::status(404);
    };
    let Ok(file) = fs::File::open(&asset.path) else {
        return AssetResponse::status(404);
    };
    let Ok(metadata) = file.metadata() else {
        return AssetResponse::status(404);
    };
    let Ok(body) = read_asset_body(file, metadata.len(), ASSET_LIMIT) else {
        return AssetResponse::status(404);
    };
    AssetResponse {
        status: 200,
        content_type: Some(asset.content_type),
        body,
    }
}

pub(crate) fn read_asset_body(
    reader: impl Read,
    declared: u64,
    limit: u64,
) -> Result<Vec<u8>, AssetError> {
    if declared > limit {
        return Err(AssetError);
    }
    let mut body = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|_| AssetError)?;
    if body.len() as u64 > limit {
        return Err(AssetError);
    }
    Ok(body)
}

fn decode_assets_path(request_path: &str) -> Result<String, AssetError> {
    if !request_path.starts_with(ASSETS_PREFIX) {
        return Err(AssetError);
    }
    let decoded = percent_decode(request_path)?;
    if !decoded.starts_with(ASSETS_PREFIX) {
        return Err(AssetError);
    }
    if decoded.contains('\0') || decoded.contains('\\') {
        return Err(AssetError);
    }
    Ok(decoded)
}

fn percent_decode(input: &str) -> Result<String, AssetError> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1).ok_or(AssetError)?;
            let low = *bytes.get(index + 2).ok_or(AssetError)?;
            decoded.push(hex_value(high)? << 4 | hex_value(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| AssetError)
}

fn hex_value(byte: u8) -> Result<u8, AssetError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(AssetError),
    }
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn content_type(file_name: &str) -> Result<&'static str, AssetError> {
    let (_, extension) = file_name.rsplit_once('.').ok_or(AssetError)?;
    match extension {
        "png" => Ok("image/png"),
        "webp" => Ok("image/webp"),
        "woff2" => Ok("font/woff2"),
        _ => Err(AssetError),
    }
}

fn refuse_symlinks(assets: &Path, segments: &[&str]) -> Result<(), AssetError> {
    let mut current = assets.to_path_buf();
    if is_symlink(&current)? {
        return Err(AssetError);
    }
    for segment in segments {
        current.push(segment);
        if is_symlink(&current)? {
            return Err(AssetError);
        }
    }
    Ok(())
}

fn is_symlink(path: &Path) -> Result<bool, AssetError> {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .map_err(|_| AssetError)
}
