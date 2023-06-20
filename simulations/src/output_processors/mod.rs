use std::time::Duration;

use chrono::{DateTime, Utc};
use consensus_engine::Overlay;
use polars::prelude::{NamedFrom, Series};
use serde::Serialize;

use crate::node::carnot::{CarnotNode, CarnotState};
use crate::node::Node;
use crate::settings::SimulationSettings;
use crate::streaming::polars::ToSeries;
use crate::warding::SimulationState;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RecordType {
    Meta,
    Settings,
    Data,
}

pub trait Record: From<Runtime> + From<SimulationSettings> + Send + Sync + 'static {
    fn record_type(&self) -> RecordType;

    fn is_settings(&self) -> bool {
        self.record_type() == RecordType::Settings
    }

    fn is_meta(&self) -> bool {
        self.record_type() == RecordType::Meta
    }

    fn is_data(&self) -> bool {
        self.record_type() == RecordType::Data
    }
}

pub type SerializedNodeState = serde_json::Value;

pub type SeriesNodeState = Vec<polars::series::Series>;

#[derive(Serialize)]
pub struct Runtime {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    elapsed: Duration,
}

impl Runtime {
    pub(crate) fn load() -> anyhow::Result<Self> {
        let elapsed = crate::START_TIME.elapsed();
        let end = Utc::now();
        Ok(Self {
            start: end
                .checked_sub_signed(chrono::Duration::from_std(elapsed)?)
                .unwrap(),
            end,
            elapsed,
        })
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum OutData<S> {
    Runtime(Runtime),
    Settings(Box<SimulationSettings>),
    Data(Box<S>),
}

impl<S> From<Runtime> for OutData<S> {
    fn from(runtime: Runtime) -> Self {
        Self::Runtime(runtime)
    }
}

impl<S> From<SimulationSettings> for OutData<S> {
    fn from(settings: SimulationSettings) -> Self {
        Self::Settings(Box::new(settings))
    }
}

impl From<CarnotState> for OutData<CarnotState> {
    fn from(state: CarnotState) -> Self {
        Self::Data(Box::new(state))
    }
}

impl<S> Record for OutData<S>
where
    S: Send + Sync + 'static,
{
    fn record_type(&self) -> RecordType {
        match self {
            Self::Runtime(_) => RecordType::Meta,
            Self::Settings(_) => RecordType::Settings,
            Self::Data(_) => RecordType::Data,
        }
    }
}

impl<S> OutData<S> {
    pub fn new(state: S) -> Self {
        Self::Data(Box::new(state))
    }
}

impl<N> TryFrom<&SimulationState<N>> for Vec<OutData<N::State>>
where
    N: Node,
    N::State: Clone,
{
    type Error = anyhow::Error;

    fn try_from(state: &crate::warding::SimulationState<N>) -> Result<Self, Self::Error> {
        Ok(state
            .nodes
            .read()
            .iter()
            .map(|n| OutData::new(n.state().clone()))
            .collect())
    }
}

impl ToSeries for OutData<CarnotState> {
    fn to_series(&self) -> Series {
        let s = match self {
            OutData::Runtime(runtime) => {
                let start = runtime.start.timestamp();
                let end = runtime.end.timestamp();
                let elapsed = runtime.elapsed.as_secs();
                Series::new("time", vec![start, end])
            }
            OutData::Settings(settings) => Series::new("settings", vec![0]),
            OutData::Data(state) => Series::new("data", vec![0]),
        };
        Series::new("OutData", vec![s])
    }
}
