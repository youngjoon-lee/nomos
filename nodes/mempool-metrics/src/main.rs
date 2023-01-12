use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use metrics::{backend::map::MapMetricsBackend, types::MetricsData, MetricsService};
use nomos_log::Logger;
use nomos_mempool::MempoolService;
use nomos_network::{backends::waku::Waku, NetworkService};
use overwatch_derive::*;
use overwatch_rs::{
    overwatch::*,
    services::{handle::ServiceHandle, ServiceData},
};
use serde::Deserialize;
use std::marker::PhantomData;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path for a yaml-encoded network config file
    config: std::path::PathBuf,
}

#[derive(Deserialize)]
struct Config {
    log: <Logger as ServiceData>::Settings,
    network: <NetworkService<Waku> as ServiceData>::Settings,
    metrics: <MetricsService<MapMetricsBackend<MetricsData>> as ServiceData>::Settings,
    mempool: MempoolService::Settings,
}

#[derive(Services)]
struct Nomos {
    logging: ServiceHandle<Logger>,
    network: ServiceHandle<NetworkService<Waku>>,
    metrics: ServiceHandle<MetricsService<MapMetricsBackend<MetricsData>>>,
    mempool: ServiceHandle<MempoolService<Waku, Mockpool>>,
    http_server: PhantomData<()>,
    api_bridge: PhantomData<()>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let Args { config } = Args::parse();
    let config = serde_yaml::from_reader::<_, Config>(std::fs::File::open(config)?)?;
    let app = OverwatchRunner::<Nomos>::run(
        NomosServiceSettings {
            network: config.network,
            logging: config.log,
            metrics: config.metrics,
            mempool: config.mempool,
            http_server: (),
            api_bridge: (),
        },
        None,
    )
    .map_err(|e| eyre!("Error encountered: {}", e))?;
    app.wait_finished();
    Ok(())
}
