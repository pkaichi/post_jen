mod agent_client;
mod agent_worker;
mod config;
mod db;
mod definition;
mod http;
mod runner;

use anyhow::Result;
use axum::serve;
use clap::Parser;
use config::Config;
use http::{AppState, router};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "postjen-server", about = "postjen job execution server")]
struct Args {
    /// Connect to a remote controller as an agent
    #[arg(long)]
    connect_to: Option<String>,

    /// Agent name when connecting to a controller
    #[arg(long, default_value = "remote-agent")]
    agent_name: String,

    /// Comma-separated agent labels when connecting to a controller
    #[arg(long, value_delimiter = ',')]
    agent_labels: Vec<String>,

    /// Agent polling interval in seconds
    #[arg(long, default_value = "2")]
    poll_interval: Option<u64>,

    /// Agent heartbeat interval in seconds
    #[arg(long, default_value = "15")]
    heartbeat_interval: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = Args::parse();
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;
    tokio::fs::create_dir_all(&config.artifacts_dir).await?;
    runner::spawn(pool.clone(), config.artifacts_dir.clone()).await?;

    // If --connect-to is specified, also run as a remote agent for that controller
    if let Some(controller_url) = &args.connect_to {
        let client = agent_client::AgentClient::new(controller_url);
        let hostname = gethostname();

        let registration = client
            .register(&args.agent_name, &hostname, &args.agent_labels)
            .await?;
        info!(
            controller = %controller_url,
            agent_id = %registration.agent_id,
            name = %args.agent_name,
            "registered as remote agent with controller"
        );

        let token = registration.token;
        let poll_interval = args.poll_interval.unwrap_or(2);
        let heartbeat_interval = args.heartbeat_interval.unwrap_or(15);

        // Spawn heartbeat
        let hb_client = client.clone();
        let hb_token = token.clone();
        tokio::spawn(async move {
            agent_worker::heartbeat_loop(&hb_client, &hb_token, heartbeat_interval).await;
        });

        // Spawn polling loop
        let poll_client = client;
        let poll_token = token;
        tokio::spawn(async move {
            agent_worker::poll_loop(&poll_client, &poll_token, poll_interval).await;
        });
    }

    let state = AppState {
        pool,
        artifacts_dir: config.artifacts_dir,
    };
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

fn gethostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}
