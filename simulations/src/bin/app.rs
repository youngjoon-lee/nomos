// std
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
// crates
use clap::Parser;
use crossbeam::channel;
use polars::io::SerWriter;
use polars::prelude::{DataFrame, JsonReader, SerReader};
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use simulations::network::behaviour::NetworkBehaviour;
use simulations::network::regions::RegionsData;
use simulations::network::{InMemoryNetworkInterface, Network};
use simulations::node::{generate_overlays, Node, NodeId, OverlayState};
use simulations::overlay::flat::FlatOverlay;
use simulations::overlay::tree::TreeOverlay;
use simulations::overlay::Overlay;
// internal
use simulations::{
    node::carnot::CarnotNode, output_processors::OutData, runner::SimulationRunner,
    settings::SimulationSettings,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
enum OutputType {
    File(PathBuf),
    StdOut,
    StdErr,
}

impl core::fmt::Display for OutputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputType::File(path) => write!(f, "{}", path.display()),
            OutputType::StdOut => write!(f, "stdout"),
            OutputType::StdErr => write!(f, "stderr"),
        }
    }
}

/// Output format selector enum
#[derive(Clone, Debug, Default)]
enum OutputFormat {
    Json,
    Csv,
    #[default]
    Parquet,
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let tag = match self {
            OutputFormat::Json => "json",
            OutputFormat::Csv => "csv",
            OutputFormat::Parquet => "parquet",
        };
        write!(f, "{tag}")
    }
}

impl FromStr for OutputFormat {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "parquet" => Ok(Self::Parquet),
            tag => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid {tag} tag, only [json, csv, polars] are supported",),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
enum OverlayType {
    Flat,
    Tree,
}
impl core::fmt::Display for OverlayType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Flat => write!(f, "flat"),
            Self::Tree => write!(f, "tree"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
enum NodeType {
    Carnot,
    Dummy,
}

impl core::fmt::Display for NodeType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Carnot => write!(f, "carnot"),
            Self::Dummy => write!(f, "dummy"),
        }
    }
}

/// Main simulation wrapper
/// Pipes together the cli arguments with the execution
#[derive(Parser)]
pub struct SimulationApp {
    /// Json file path, on `SimulationSettings` format
    #[clap(long, short)]
    input_settings: PathBuf,
    #[clap(long, default_value_t = NodeType::Carnot)]
    node_type: NodeType,
    /// Output file path
    #[clap(long, short)]
    output_file: PathBuf,
    /// Output format selector
    #[clap(long, short = 'f', default_value_t)]
    output_format: OutputFormat,
    #[clap(long, default_value_t = OverlayType::Tree)]
    overlay_type: OverlayType,
}

impl SimulationApp {
    pub fn run(self) -> anyhow::Result<()> {
        let Self {
            input_settings,
            node_type,
            output_file,
            output_format,
            overlay_type,
        } = self;
        let mut simulation_runner = match (node_type, overlay_type) {
            (NodeType::Carnot, OverlayType::Flat) => {
                setup::<(), CarnotNode, FlatOverlay>(input_settings)?
            }
            (NodeType::Carnot, OverlayType::Tree) => todo!(),
            (NodeType::Dummy, OverlayType::Flat) => todo!(),
            (NodeType::Dummy, OverlayType::Tree) => todo!(),
        };
        // build up series vector
        let mut out_data: Vec<OutData> = Vec::new();
        simulation_runner.simulate(Some(&mut out_data))?;
        let mut dataframe: DataFrame = out_data_to_dataframe(out_data);
        dump_dataframe_to(output_format, &mut dataframe, &output_file)?;
        Ok(())
    }
}

fn setup<M, N, O>(input_settings: PathBuf) -> anyhow::Result<SimulationRunner<M, N, O>>
where
    M: Clone,
    N: Node + Send + Sync,
    N::Settings: DeserializeOwned + Clone,
    N::State: Serialize,
    O: Overlay,
    O::Settings: DeserializeOwned + Clone,
{
    let mut rng = SmallRng::seed_from_u64(1234);
    let simulation_settings: SimulationSettings<N::Settings, O::Settings> =
        load_json_from_file(&input_settings)?;
    let mut node_ids: Vec<NodeId> = (0..simulation_settings.node_count)
        .map(Into::into)
        .collect();
    node_ids.shuffle(&mut rng);

    let mut region_nodes = node_ids.clone();
    let regions = simulation_settings
        .regions
        .iter()
        .map(|(region, distribution)| {
            // Node ids should be already shuffled.
            let node_count = (node_ids.len() as f32 * distribution).round() as usize;
            let nodes = region_nodes.drain(..node_count).collect::<Vec<_>>();
            (*region, nodes)
        })
        .collect();
    let behaviours = simulation_settings
        .network_behaviors
        .iter()
        .map(|((a, b), d)| ((*a, *b), NetworkBehaviour::new(*d, 0.0)))
        .collect();
    let regions_data = RegionsData::new(regions, behaviours);
    let overlay = O::new(simulation_settings.overlay_settings.clone());
    let overlays = generate_overlays(
        &node_ids,
        overlay,
        simulation_settings.overlay_count,
        simulation_settings.leader_count,
        &mut rng,
    );
    let overlay_state = Arc::new(RwLock::new(OverlayState {
        all_nodes: node_ids.clone(),
        overlays,
    }));

    let network = Network::new(regions_data);

    let nodes = vec![];
    // node_ids
    // .iter()
    // .map(|node_id| {
    //     let (node_message_sender, node_message_receiver) = channel::unbounded();
    //     let network_message_receiver = network.connect(*node_id, node_message_receiver);
    //     let network_interface = InMemoryNetworkInterface::new(
    //         *node_id,
    //         node_message_sender,
    //         network_message_receiver,
    //     );
    //     (
    //         *node_id,
    //         N::new(*node_id, 0, overlay_state.clone(), network_interface),
    //     )
    // })
    // .collect();
    Ok(SimulationRunner::new(network, nodes, simulation_settings))
}

fn out_data_to_dataframe(out_data: Vec<OutData>) -> DataFrame {
    let mut cursor = Cursor::new(Vec::new());
    serde_json::to_writer(&mut cursor, &out_data).expect("Dump data to json ");
    let dataframe = JsonReader::new(cursor)
        .finish()
        .expect("Load dataframe from intermediary json");

    dataframe
        .unnest(["state"])
        .expect("Node state should be unnest")
}

/// Generically load a json file
fn load_json_from_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let f = File::open(path).map_err(Box::new)?;
    Ok(serde_json::from_reader(f)?)
}

fn dump_dataframe_to_json(data: &mut DataFrame, out_path: &Path) -> anyhow::Result<()> {
    let out_path = out_path.with_extension("json");
    let f = File::create(out_path)?;
    let mut writer = polars::prelude::JsonWriter::new(f);
    Ok(writer.finish(data)?)
}

fn dump_dataframe_to_csv(data: &mut DataFrame, out_path: &Path) -> anyhow::Result<()> {
    let out_path = out_path.with_extension("csv");
    let f = File::create(out_path)?;
    let mut writer = polars::prelude::CsvWriter::new(f);
    Ok(writer.finish(data)?)
}

fn dump_dataframe_to_parquet(data: &mut DataFrame, out_path: &Path) -> anyhow::Result<()> {
    let out_path = out_path.with_extension("parquet");
    let f = File::create(out_path)?;
    let writer = polars::prelude::ParquetWriter::new(f);
    Ok(writer.finish(data).map(|_| ())?)
}

fn dump_dataframe_to(
    output_format: OutputFormat,
    data: &mut DataFrame,
    out_path: &Path,
) -> anyhow::Result<()> {
    match output_format {
        OutputFormat::Json => dump_dataframe_to_json(data, out_path),
        OutputFormat::Csv => dump_dataframe_to_csv(data, out_path),
        OutputFormat::Parquet => dump_dataframe_to_parquet(data, out_path),
    }
}

fn main() -> anyhow::Result<()> {
    let app: SimulationApp = SimulationApp::parse();
    app.run()?;
    Ok(())
}
