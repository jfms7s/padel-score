//! Serves the SvelteKit app, built to static files and embedded in the binary.
//! Unknown paths get index.html so client-side routes work on reload.

use axum::Json;
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/build/"]
struct Assets;

/// SvelteKit puts hashed files under _app/immutable; everything else must revalidate.
pub fn cache_control(path: &str) -> &'static str {
    if path.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn is_api(path: &str) -> bool {
    path == "api" || path.starts_with("api/")
}

/// A path that names a file (under `_app/` or with an extension) must never fall back to
/// index.html, or a stale chunk would be served HTML after a deploy.
fn is_asset_path(path: &str) -> bool {
    path.starts_with("_app/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
}

pub async fn static_handler(method: Method, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if is_api(path) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CACHE_CONTROL, cache_control(path).to_string()),
            ],
            file.data,
        )
            .into_response();
    }
    if is_asset_path(path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get("index.html") {
        Some(index) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            index.data,
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "The web app wasn't built into this binary (run npm run build in web/).",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_asset_gets_long_cache() {
        assert_eq!(
            cache_control("_app/immutable/chunks/a.js"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("_app/version.json"), "no-cache");
    }
}
