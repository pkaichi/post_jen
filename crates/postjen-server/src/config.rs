use std::{env, net::SocketAddr};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("POSTJEN_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
            .parse()
            .context("failed to parse POSTJEN_BIND_ADDR")?;

        let database_url =
            env::var("POSTJEN_DATABASE_URL").unwrap_or_else(|_| "sqlite:postjen.db".to_string());

        Ok(Self {
            bind_addr,
            database_url,
        })
    }
}
