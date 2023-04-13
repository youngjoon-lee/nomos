use crate::network::regions::Region;
use crate::warding::Ward;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, Default)]
pub enum RunnerSettings {
    #[default]
    Sync,
    Async {
        chunks: usize,
    },
    Glauber {
        maximum_iterations: usize,
        update_rate: usize,
    },
    Layered {
        rounds_gap: usize,
        distribution: Option<Vec<f32>>,
    },
}

#[derive(Default, Deserialize)]
pub struct SimulationSettings<N, O> {
    pub network_behaviors: HashMap<(Region, Region), Duration>,
    /// Represents node distribution in the simulated regions.
    /// The sum of distributions should be 1.
    pub regions: HashMap<Region, f32>,
    #[serde(default)]
    pub wards: Vec<Ward>,
    pub overlay_settings: O,
    pub node_settings: N,
    pub runner_settings: RunnerSettings,
    pub node_count: usize,
    pub committee_size: usize,
    pub leader_count: usize,
    pub overlay_count: usize,
    pub seed: Option<u64>,
}
