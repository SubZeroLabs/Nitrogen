use anyhow::Context;
use log::{error, info};
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use tokio::net::TcpListener;

pub(crate) mod mc_types;
mod incoming_client;

const CURRENT_PROTOCOL: (i32, &str) = (756, "1.17.1");

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub struct Network {
    bind: String,
    port: u16,
    compression_threshold: i32,
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
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .apply()
        .context("Failed to apply configuration to log dispatcher.")?;
    Ok(())
}

async fn read_config() -> anyhow::Result<Config> {
    let mut file = File::open("./Config.toml").context(format!(
        "Failed to open configuration file: {}/Config.toml",
        std::env::current_dir()?.to_str().unwrap_or("Unknown")
    ))?;
    let mut contents = vec![];
    file.read_to_end(&mut contents)
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
    let config = Arc::new(read_config().await?);
    info!("Starting server with configuration {:#?}", &config);
    let bind = format!("{}:{}", config.network.bind, config.network.port);
    info!("Binding to tokio listener on {}", &bind);
    let listener = TcpListener::bind(&bind).await?;
    info!("Proxy Started: Listening on {}", &bind);
    watch_incoming(listener, &config).await;
    Ok(())
}

async fn watch_incoming(listener: TcpListener, config: &Arc<Config>) {
    let config = Arc::clone(config);
    loop {
        if watch_for_client(&listener, Arc::clone(&config)).await.is_err() {
            panic!("Something went wrong taking on a new client!");
        }
    }
}

async fn watch_for_client(listener: &TcpListener, next_config: Arc<Config>) -> anyhow::Result<()> {
    let (socket, address) = listener.accept().await?;
    tokio::spawn(async move {
        let arc_address = Arc::new(address);
        if let Err(e) = incoming_client::accept_client(socket, Arc::clone(&arc_address), next_config).await {
            log::error!(target: &arc_address.to_string(), "Incoming client fell into error {:?}", e);
        }
    });
    Ok(())
}
