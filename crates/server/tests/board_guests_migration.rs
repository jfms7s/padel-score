#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::str::FromStr;

use padel_score_server::db;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

async fn fk_tables(pool: &SqlitePool, table: &str) -> Vec<(String, String)> {
    sqlx::query(r#"SELECT "table", on_delete FROM pragma_foreign_key_list(?)"#)
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

async fn insert_user(pool: &SqlitePool, id: &str, name: &str) {
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash, created_at) VALUES (?, ?, ?, 'x', 1)",
    )
    .bind(id)
    .bind(format!("{id}@x.test"))
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_board(pool: &SqlitePool, user: &str) {
    sqlx::query(
        "INSERT INTO boards (id, token, creator_user_id, title, time_zone, starts_at, ends_at, created_at) \
         VALUES ('b1', 't1', ?, 'T', 'Europe/Lisbon', 10, 20, 1)",
    )
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_0014_drops_the_users_fk() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    for table in ["board_members", "board_game_players"] {
        let fks = fk_tables(&pool, table).await;
        assert!(fks.iter().all(|(t, _)| t != "users"), "{table}: {fks:?}");
    }
    let ranges = fk_tables(&pool, "board_ranges").await;
    assert!(!ranges.is_empty(), "{ranges:?}");
    assert!(
        ranges
            .iter()
            .all(|(t, d)| t == "board_members" && d == "CASCADE"),
        "{ranges:?}"
    );
}

#[tokio::test]
async fn guest_rows_need_the_prefix() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    insert_user(&pool, "u1", "Ana").await;
    insert_board(&pool, "u1").await;
    let err = sqlx::query(
        "INSERT INTO board_guests (id, board_id, name, name_key, created_at) VALUES ('nope', 'b1', 'G', 'g', 1)",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("CHECK"), "{err}");
}

#[tokio::test]
async fn board_people_names_users_and_guests() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    insert_user(&pool, "u1", "Ana").await;
    insert_board(&pool, "u1").await;
    sqlx::query(
        "INSERT INTO board_guests (id, board_id, name, name_key, created_at) VALUES ('guest_1', 'b1', 'Gus', 'gus', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (id, want) in [("u1", "Ana"), ("guest_1", "Gus")] {
        let name: String = sqlx::query_scalar("SELECT name FROM board_people WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(name, want);
    }
}

#[tokio::test]
async fn migration_0014_keeps_existing_board_rows() {
    let dir = std::env::temp_dir().join(format!("migrations-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if name.as_str() < "0014" {
            std::fs::copy(&path, dir.join(&name)).unwrap();
        }
    }
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate::Migrator::new(dir.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    insert_user(&pool, "u1", "Ana").await;
    insert_board(&pool, "u1").await;
    for sql in [
        "INSERT INTO board_members (board_id, user_id, joined_at) VALUES ('b1', 'u1', 5)",
        "INSERT INTO board_ranges (id, board_id, user_id, starts_at, ends_at, created_at) VALUES ('r1', 'b1', 'u1', 11, 12, 6)",
        "INSERT INTO board_games (id, board_id, starts_at, ends_at, formed_at) VALUES ('g1', 'b1', 11, 12, 7)",
        "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) VALUES ('g1', 'u1', 2, 8)",
        "INSERT INTO board_line_heads (game_id, user_id, at) VALUES ('g1', 'u1', 9)",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let member: (String, String, i64) =
        sqlx::query_as("SELECT board_id, user_id, joined_at FROM board_members")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(member, ("b1".into(), "u1".into(), 5));
    let range: (String, String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT id, board_id, user_id, starts_at, ends_at, created_at FROM board_ranges",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(range, ("r1".into(), "b1".into(), "u1".into(), 11, 12, 6));
    let seat: (String, String, i64, i64) =
        sqlx::query_as("SELECT game_id, user_id, seat, joined_at FROM board_game_players")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(seat, ("g1".into(), "u1".into(), 2, 8));
    let head: (String, String, i64) =
        sqlx::query_as("SELECT game_id, user_id, at FROM board_line_heads")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(head, ("g1".into(), "u1".into(), 9));
    let game: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_games WHERE id = 'g1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(game, 1);

    sqlx::query("DELETE FROM board_members WHERE user_id = 'u1'")
        .execute(&pool)
        .await
        .unwrap();
    let ranges: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_ranges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ranges, 0);
}
