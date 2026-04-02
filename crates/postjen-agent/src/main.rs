mod client;
mod worker;

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "postjen-agent", about = "postjen remote execution agent")]
struct Args {
    /// Server URL (e.g. http://127.0.0.1:3000)
    #[arg(long)]
    server_url: String,

    /// Agent display name
    #[arg(long)]
    name: String,

    /// Comma-separated labels for task matching
    #[arg(long, value_delimiter = ',')]
    labels: Vec<String>,

    /// Polling interval in seconds
    #[arg(long, default_value = "2")]
    poll_interval: u64,

    /// Heartbeat interval in seconds
    #[arg(long, default_value = "15")]
    heartbeat_interval: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();
    info!(
        name = %args.name,
        labels = ?args.labels,
        server = %args.server_url,
        "starting postjen-agent"
    );

    let client = client::AgentClient::new(&args.server_url);

    // Register with server
    let registration = client.register(&args.name, &hostname(), &args.labels).await?;
    info!(
        agent_id = %registration.agent_id,
        "registered with server"
    );

    let token = registration.token;

    // Spawn heartbeat task
    let heartbeat_client = client.clone();
    let heartbeat_token = token.clone();
    let heartbeat_interval = args.heartbeat_interval;
    tokio::spawn(async move {
        worker::heartbeat_loop(&heartbeat_client, &heartbeat_token, heartbeat_interval).await;
    });

    // Run polling loop
    worker::poll_loop(&client, &token, args.poll_interval).await;

    Ok(())
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| {
            gethostname().unwrap_or_else(|| "unknown".to_string())
        })
}

fn gethostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let ret = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
        if ret == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            Some(String::from_utf8_lossy(&buf[..end]).to_string())
        } else {
            None
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
}
