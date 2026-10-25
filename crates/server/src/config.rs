use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;

use ipnet::IpNet;

/// Runtime configuration, read from the environment. Blank values count as unset.
///
/// `Debug` is written by hand so the setup token never reaches a log.
#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub database_url: String,
    pub port: u16,
    pub log_level: String,
    pub version: String,
    pub commit: String,
    /// `SECURE_COOKIES`: on unless set to `false` or `0`. On, the session cookie is
    /// `__Host-padel_session` with `Secure`; off (plain-HTTP development) it is `padel_session`.
    pub secure_cookies: bool,
    /// `PUBLIC_ORIGIN`, e.g. `https://padel.example.com`, without a trailing slash. The CSRF guard
    /// compares a request's `Origin` with it; unset, it compares the `Origin` host with `Host`.
    pub public_origin: Option<String>,
    /// `SETUP_TOKEN`: the code that creates the first admin. Unset, a one-time code is logged.
    pub setup_token: Option<String>,
    /// `TRUSTED_PROXIES`: comma-separated CIDRs or bare IPs of the reverse proxies whose
    /// `X-Forwarded-For` is believed (see [`crate::client_ip`]). Empty by default: the TCP peer
    /// is the client.
    pub trusted_proxies: Vec<IpNet>,
}

/// A configuration value that cannot be used. Startup stops on it rather than guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A `TRUSTED_PROXIES` entry that is neither a CIDR nor an IP address.
    TrustedProxy(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::TrustedProxy(entry) => write!(
                f,
                "TRUSTED_PROXIES entry {entry:?} is not a CIDR or an IP address"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Parse `TRUSTED_PROXIES`: comma-separated CIDRs (`10.42.0.0/16`) or bare IPs (`192.0.2.1`,
/// taken as a /32 or /128). Blank entries are skipped; any other bad entry is an error.
pub fn parse_trusted_proxies(raw: &str) -> Result<Vec<IpNet>, ConfigError> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .parse::<IpNet>()
                .or_else(|_| {
                    entry
                        .parse::<IpAddr>()
                        .map(|ip| IpNet::from(ip.to_canonical()))
                })
                .map(|net| net.trunc())
                .map_err(|_| ConfigError::TrustedProxy(entry.to_owned()))
        })
        .collect()
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("database_url", &self.database_url)
            .field("port", &self.port)
            .field("log_level", &self.log_level)
            .field("version", &self.version)
            .field("commit", &self.commit)
            .field("secure_cookies", &self.secure_cookies)
            .field("public_origin", &self.public_origin)
            .field("trusted_proxies", &self.trusted_proxies)
            .field(
                "setup_token",
                &self.setup_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = std::env::vars().collect();
        Config::from_map(&map)
    }

    /// Read the configuration from `map`. Unparseable optional values fall back to their
    /// defaults, except a bad `TRUSTED_PROXIES` entry, which is an error: silently trusting fewer
    /// proxies than intended would key every client behind the proxy as one address.
    pub fn from_map(map: &HashMap<String, String>) -> Result<Config, ConfigError> {
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
        let trusted_proxies = parse_trusted_proxies(&get("TRUSTED_PROXIES").unwrap_or_default())?;
        Ok(Config {
            data_dir,
            database_url,
            port: get("PORT").and_then(|p| p.parse().ok()).unwrap_or(8080),
            log_level: get("LOG_LEVEL").unwrap_or_else(|| "info,sqlx=warn".into()),
            version: get("APP_VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            commit: get("GIT_COMMIT").unwrap_or_else(|| "unknown".into()),
            secure_cookies: !get("SECURE_COOKIES")
                .is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("false")),
            public_origin: get("PUBLIC_ORIGIN")
                .map(|v| v.trim_end_matches('/').to_owned())
                .filter(|v| !v.is_empty()),
            setup_token: get("SETUP_TOKEN"),
            trusted_proxies,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> Config {
        let map = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_map(&map).unwrap()
    }

    #[test]
    fn trusted_proxies_default_empty_and_accept_cidrs_and_bare_ips() {
        assert!(config(&[]).trusted_proxies.is_empty());
        assert!(
            config(&[("TRUSTED_PROXIES", "  ")])
                .trusted_proxies
                .is_empty()
        );
        let c = config(&[(
            "TRUSTED_PROXIES",
            "10.42.0.0/16, 192.0.2.1 ,2001:db8::/32,::1,",
        )]);
        let shown: Vec<String> = c.trusted_proxies.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            ["10.42.0.0/16", "192.0.2.1/32", "2001:db8::/32", "::1/128"]
        );
    }

    #[test]
    fn bad_trusted_proxies_entry_is_a_config_error() {
        for bad in [
            "10.42.0.0/33",
            "not-a-net",
            "10.42.0.0/16, 300.1.1.1",
            "10.0.0.1:80",
        ] {
            let map = HashMap::from([("TRUSTED_PROXIES".to_owned(), bad.to_owned())]);
            let Err(error) = Config::from_map(&map) else {
                panic!("{bad:?} was accepted");
            };
            let shown = error.to_string();
            assert!(shown.contains("TRUSTED_PROXIES"), "{shown}");
            let offending = bad.split(',').next_back().unwrap().trim();
            let offending = if bad.starts_with("10.42.0.0/16,") {
                offending
            } else {
                bad
            };
            assert!(shown.contains(offending), "{shown}");
        }
    }

    #[test]
    fn secure_cookies_default_on_and_only_false_or_0_turn_it_off() {
        assert!(config(&[]).secure_cookies);
        for on in ["true", "1", "yes", "", "  ", "nonsense"] {
            assert!(config(&[("SECURE_COOKIES", on)]).secure_cookies, "{on:?}");
        }
        for off in ["false", "0", "FALSE", " False "] {
            assert!(
                !config(&[("SECURE_COOKIES", off)]).secure_cookies,
                "{off:?}"
            );
        }
    }

    #[test]
    fn public_origin_and_setup_token_are_optional() {
        let c = config(&[]);
        assert_eq!(c.public_origin, None);
        assert_eq!(c.setup_token, None);
        let c = config(&[
            ("PUBLIC_ORIGIN", " https://padel.example.test/ "),
            ("SETUP_TOKEN", "s3cret-setup"),
        ]);
        assert_eq!(
            c.public_origin.as_deref(),
            Some("https://padel.example.test")
        );
        assert_eq!(c.setup_token.as_deref(), Some("s3cret-setup"));
        let c = config(&[("PUBLIC_ORIGIN", "  "), ("SETUP_TOKEN", "")]);
        assert_eq!(c.public_origin, None);
        assert_eq!(c.setup_token, None);
    }

    #[test]
    fn debug_redacts_the_setup_token() {
        let c = config(&[("SETUP_TOKEN", "s3cret-setup")]);
        let shown = format!("{c:?}");
        assert!(!shown.contains("s3cret-setup"), "{shown}");
        assert!(shown.contains("setup_token"), "{shown}");
    }
}
