use std::collections::HashMap;
use std::path::PathBuf;

/// Runtime configuration, read from the environment. Blank values count as unset.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub database_url: String,
    pub port: u16,
    pub log_level: String,
    pub version: String,
    pub commit: String,
}

impl Config {
    pub fn from_env() -> Config {
        let map: HashMap<String, String> = std::env::vars().collect();
        Config::from_map(&map)
    }

    pub fn from_map(map: &HashMap<String, String>) -> Config {
        let get = |key: &str| {
            map.get(key)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let data_dir = PathBuf::from(get("DATA_DIR").unwrap_or_else(|| "/data".into()));
        let database_url = get("DATABASE_URL").unwrap_or_else(|| {
            format!(
                "sqlite://{}?mode=rwc",
                data_dir.join("padel-score.db").display()
            )
        });
        Config {
            data_dir,
            database_url,
            port: get("PORT").and_then(|p| p.parse().ok()).unwrap_or(8080),
            log_level: get("LOG_LEVEL").unwrap_or_else(|| "info,sqlx=warn".into()),
            version: get("APP_VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            commit: get("GIT_COMMIT").unwrap_or_else(|| "unknown".into()),
        }
    }
}
