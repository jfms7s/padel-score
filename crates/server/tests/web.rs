#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use padel_score_server::{AppState, Config, router};
use tower::ServiceExt;

fn app() -> axum::Router {
    router(AppState::new(Arc::new(Config::from_map(&HashMap::new()))))
}

async fn get(path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = app()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, body)
}

#[tokio::test]
async fn health_reports_ok_and_version() {
    let (status, _, body) = get("/api/health").await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert!(v["version"].is_string());
    assert!(v["commit"].is_string());
}

#[tokio::test]
async fn unknown_api_path_is_json_404() {
    let (status, headers, body) = get("/api/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("json")
    );
    assert!(String::from_utf8(body).unwrap().contains("\"not_found\""));
}

#[tokio::test]
async fn unknown_page_falls_back_to_index() {
    let (status, headers, _) = get("/s/abc").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[test]
fn config_defaults() {
    let c = Config::from_map(&HashMap::new());
    assert_eq!(c.port, 8080);
    assert_eq!(c.data_dir, std::path::PathBuf::from("/data"));
    assert_eq!(c.database_url, "sqlite:///data/padel-score.db?mode=rwc");
    assert_eq!(c.log_level, "info,sqlx=warn");
}

#[test]
fn config_blank_values_count_as_unset() {
    let map: HashMap<String, String> = [("PORT", "  "), ("DATA_DIR", "/tmp/x")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let c = Config::from_map(&map);
    assert_eq!(c.port, 8080);
    assert_eq!(c.database_url, "sqlite:///tmp/x/padel-score.db?mode=rwc");
}
