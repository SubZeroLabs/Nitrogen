#![feature(in_band_lifetimes)]

use anyhow::Context;
use log::{error, info};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

mod cs_relationship;

const CURRENT_PROTOCOL: (i32, &str) = (756, "1.17.1");

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub struct Network {
    bind: String,
    port: u16,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub struct ServerInfo {
    motd: String,
    max_players: usize,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub struct Config {
    network: Network,
    server_info: ServerInfo,
}

fn setup_logger() -> anyhow::Result<()> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} ({}) => {}: {}",
                chrono::Local::now().format("%Y/%m/%d | %H:%M:%S"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Trace)
        .chain(std::io::stdout())
        .apply()
        .context("Failed to apply configuration to log dispatcher.")?;
    Ok(())
}

async fn read_config() -> anyhow::Result<Config> {
    let mut file = File::open("./Config.toml").await.context(format!(
        "Failed to open configuration file: {}/Config.toml",
        std::env::current_dir()?.to_str().unwrap_or("Unknown")
    ))?;
    let mut contents = vec![];
    file.read_to_end(&mut contents)
        .await
        .context("Failed to read config to internal string.")?;
    toml::from_slice::<Config>(&contents).context("Failed to read configuration.")
}

#[tokio::main]
async fn main() {
    if let Err(e) = setup_proxy().await {
        error!("Fatal error within proxy: {}", e);
    }
}

async fn setup_proxy() -> anyhow::Result<()> {
    println!("Setting up log dispatcher.");
    setup_logger()?;
    info!(
        "Reading config from {}/Config.toml",
        std::env::current_dir()?.to_str().unwrap_or("Unknown")
    );
    let config = Arc::new(Mutex::new(read_config().await?));
    let local = config.lock().await;
    let bind = format!("{}:{}", local.network.bind, local.network.port);
    info!("Binding to tokio listener on {}", &bind);
    let listener = TcpListener::bind(&bind).await?;
    info!("Proxy Started: Listening on {}", &bind);
    drop(local);
    loop {
        let (socket, address) = listener.accept().await?;
        let config_client_copy = Arc::clone(&config);
        tokio::spawn(async move {
            cs_relationship::incoming_client::new_client(socket, address, config_client_copy).await;
        });
    }
}
