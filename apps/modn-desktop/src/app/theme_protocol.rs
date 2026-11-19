//! The `modn-theme` URI scheme.
//!
//! It serves only files the asset rules accept from the root `resolve_theme`
//! stored. Every refusal is an empty 404, so the webview never learns why a
//! request failed. Fonts are fetched in CORS mode, so every answer allows any
//! origin.

use super::state::AppState;
use super::theme_assets::{AssetResponse, load_asset};
use tauri::Manager;

/// Registers the scheme on the application builder.
pub(super) fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_uri_scheme_protocol("modn-theme", |context, request| {
        let state = context.app_handle().state::<AppState>();
        let root = state.theme_root();
        let asset = load_asset(
            root.as_deref(),
            request.method().as_str(),
            request.uri().path(),
        );
        response(asset)
    })
}

/// Adds the headers the webview needs to every answer.
fn response(asset: AssetResponse) -> tauri::http::Response<Vec<u8>> {
    let mut builder = tauri::http::Response::builder()
        .status(asset.status)
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*");
    if let Some(content_type) = asset.content_type {
        builder = builder.header("Content-Type", content_type);
    }
    builder
        .body(asset.body)
        .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::response;
    use crate::app::theme_assets::AssetResponse;

    #[test]
    fn a_served_asset_carries_its_type_and_the_cors_headers() {
        let response = response(AssetResponse {
            status: 200,
            content_type: Some("font/woff2"),
            body: b"wOF2".to_vec(),
        });
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["Content-Type"], "font/woff2");
        assert_eq!(response.headers()["Access-Control-Allow-Origin"], "*");
        assert_eq!(response.headers()["Cache-Control"], "no-store");
        assert_eq!(response.body(), b"wOF2");
    }

    #[test]
    fn a_refusal_answers_404_with_an_empty_body_and_no_type() {
        let response = response(AssetResponse {
            status: 404,
            content_type: None,
            body: Vec::new(),
        });
        assert_eq!(response.status(), 404);
        assert!(response.headers().get("Content-Type").is_none());
        assert_eq!(response.headers()["Access-Control-Allow-Origin"], "*");
        assert!(response.body().is_empty());
    }

    #[test]
    fn a_refused_method_answers_405() {
        let response = response(AssetResponse {
            status: 405,
            content_type: None,
            body: Vec::new(),
        });
        assert_eq!(response.status(), 405);
        assert!(response.body().is_empty());
    }
}
