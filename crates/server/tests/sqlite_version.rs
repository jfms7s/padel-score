#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The SQLite the server links must carry the fix for RUSTSEC-2026-0459 (CVE-2026-119375):
//! before 3.54.0 a crafted `-wal` file of another page size could expose uninitialized
//! page-cache memory. This guards the bundled version against a lockfile regression.

use padel_score_server::db;

#[tokio::test]
async fn bundled_sqlite_is_at_least_3_54() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let version: String = sqlx::query_scalar("select sqlite_version()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
    let (major, minor, patch) = (parts[0], parts[1], parts.get(2).copied().unwrap_or(0));
    assert!(
        (major, minor, patch) >= (3, 54, 0),
        "linked SQLite is {version}, need at least 3.54.0"
    );
}
