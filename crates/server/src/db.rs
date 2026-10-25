//! SQLite pool and migrations.

use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

fn is_memory(url: &str) -> bool {
    url.starts_with("sqlite::memory:") || url.contains("mode=memory")
}

/// Open the database, apply the pragmas and run the migrations.
///
/// An in-memory database lives and dies with its connection, so it gets exactly one that is
/// never recycled. WAL does not apply there.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let memory = is_memory(url);
    let mut options = SqliteConnectOptions::from_str(url)?
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5000));
    if !memory {
        options = options.journal_mode(SqliteJournalMode::Wal);
    }
    let mut pool = SqlitePoolOptions::new();
    pool = if memory {
        pool.max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
    } else {
        pool.max_connections(4)
    };
    let pool = pool.connect_with(options).await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
