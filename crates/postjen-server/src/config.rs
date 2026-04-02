use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub artifacts_dir: PathBuf,
    pub secret_key: Option<Vec<u8>>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("POSTJEN_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
            .parse()
            .context("failed to parse POSTJEN_BIND_ADDR")?;

        let database_url =
            env::var("POSTJEN_DATABASE_URL").unwrap_or_else(|_| "sqlite:postjen.db".to_string());

        let artifacts_dir = PathBuf::from(
            env::var("POSTJEN_ARTIFACTS_DIR").unwrap_or_else(|_| "artifacts".to_string()),
        );

        let secret_key = env::var("POSTJEN_SECRET_KEY").ok().and_then(|hex_str| {
            hex::decode(hex_str.trim()).ok().filter(|bytes| bytes.len() == 32)
        });

        Ok(Self {
            bind_addr,
            database_url,
            artifacts_dir,
            secret_key,
        })
    }
}
