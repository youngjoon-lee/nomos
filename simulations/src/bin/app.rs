use std::{
    fs::File,
    path::{Path, PathBuf},
};

use clap::Parser;
use serde::de::DeserializeOwned;
use simulations::{app::SimulationApp, settings::SimulationSettings, streaming::StreamType};

/// Pipes together the cli arguments with the execution
#[derive(Parser)]
pub struct Config {
    /// Json file path, on `SimulationSettings` format
    #[clap(long, short)]
    input_settings: PathBuf,
    #[clap(long)]
    stream_type: StreamType,
}

fn main() -> anyhow::Result<()> {
    let Config {
        input_settings,
        stream_type,
    } = Config::parse();
    let simulation_settings: SimulationSettings = load_json_from_file(&input_settings)?;
    SimulationApp::run(simulation_settings, stream_type)?;
    Ok(())
}

/// Generically load a json file
fn load_json_from_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let f = File::open(path).map_err(Box::new)?;
    Ok(serde_json::from_reader(f)?)
}
