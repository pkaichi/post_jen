mod config;
mod db;
mod definition;
mod http;
mod runner;

use anyhow::Result;
use axum::serve;
use config::Config;
use http::{AppState, router};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;
    runner::spawn(pool.clone());
    let state = AppState { pool };
    let app = router(state);

    let listener = TcpListener::bind(config.bind_addr).await?;
    info!("listening on {}", listener.local_addr()?);

    serve(listener, app).await?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt().with_env_filter(filter).init();
}
