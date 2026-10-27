use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use padel_score_server::{AppState, Config, Store, db, router, trace_layer};
use tower_http::compression::CompressionLayer;
use tracing_subscriber::EnvFilter;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            // Logging is configured from the config, so this goes straight to stderr.
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };
    if let Ok(raw) = std::env::var("PORT")
        && !raw.trim().is_empty()
        && raw.trim().parse::<u16>().is_err()
    {
        eprintln!(
            "warning: PORT {raw:?} is not a valid port, using {}",
            config.port
        );
    }
    // `padel-score healthcheck` is the container HEALTHCHECK (the image has no curl or wget).
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return match tokio::time::timeout(Duration::from_secs(3), healthcheck(config.port)).await {
            Ok(result) => result,
            Err(_) => Err("healthcheck timed out after 3s".into()),
        };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let trusted_proxies = config.trusted_proxies.len();
    if trusted_proxies == 0 {
        tracing::warn!(
            "no TRUSTED_PROXIES: the TCP peer is the client IP; behind a reverse proxy every client shares its key"
        );
    } else {
        tracing::info!(trusted_proxies, "trusted proxies configured");
    }
    let port = config.port;
    if let Some(dir) = config
        .database_url
        .strip_prefix("sqlite://")
        .and_then(|rest| std::path::Path::new(rest.split('?').next().unwrap_or(rest)).parent())
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let store = Store::new(db::connect(&config.database_url).await?);
    let state = AppState::new(Arc::new(config), store);
    // Logs the one-time setup code while there are no users and SETUP_TOKEN is unset.
    padel_score_server::accounts::init_setup_code(&state).await?;
    // Hash the dummy now, so the first unknown-email login is not a timing outlier. A server
    // that cannot hash fails closed instead of serving logins.
    if let Err(error) = padel_score_server::auth::warm_dummy(&state.password_permits).await {
        tracing::error!(%error, "cannot start");
        std::process::exit(1);
    }
    // History reads `match_results`; fill it for matches finished before it existed. A failure
    // only leaves those old matches out of history until the next boot.
    match padel_score_server::results::backfill(&state.store).await {
        Ok(inserted) => tracing::info!(inserted, "match results backfilled"),
        Err(error) => tracing::error!(%error, "match results backfill failed"),
    }
    let streams = state.shutdown.clone();
    tokio::spawn(padel_score_server::retention::run(state.clone()));
    tokio::spawn(padel_score_server::non_stop_api::run_clock(state.clone()));
    tokio::spawn(padel_score_server::push::run_sender(state.clone()));
    tokio::spawn(padel_score_server::tick::run(state.clone()));
    tokio::spawn(padel_score_server::ratelimit::run_sweeper(
        state.limits.clone(),
        state.shutdown.clone(),
    ));
    let app = router(state)
        .layer(CompressionLayer::new())
        .layer(trace_layer());
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Connect info gives the handlers the TCP peer, the base of the client IP (`client_ip`).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        // End the SSE streams too, or they would hold graceful shutdown open.
        streams.cancel();
    })
    .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "cannot install ctrl-c handler");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

async fn healthcheck(port: u16) -> Result<(), BoxError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
    stream
        .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let head = String::from_utf8_lossy(&buf);
    if head.starts_with("HTTP/1.0 200") || head.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(format!("unhealthy: {}", head.lines().next().unwrap_or("")).into())
    }
}
