use std::time::Duration;

use chrono::{DateTime, Utc};
use polars::prelude::{NamedFrom, Series};
use serde::Serialize;

use crate::node::carnot::CarnotState;
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
pub enum OutData {
    Runtime(Runtime),
    Settings(Box<SimulationSettings>),
    Data(Box<CarnotState>),
}

impl From<Runtime> for OutData {
    fn from(runtime: Runtime) -> Self {
        Self::Runtime(runtime)
    }
}

impl From<SimulationSettings> for OutData {
    fn from(settings: SimulationSettings) -> Self {
        Self::Settings(Box::new(settings))
    }
}

impl From<CarnotState> for OutData {
    fn from(state: CarnotState) -> Self {
        Self::Data(Box::new(state))
    }
}

impl Record for OutData {
    fn record_type(&self) -> RecordType {
        match self {
            Self::Runtime(_) => RecordType::Meta,
            Self::Settings(_) => RecordType::Settings,
            Self::Data(_) => RecordType::Data,
        }
    }
}

impl OutData {
    pub fn new(state: CarnotState) -> Self {
        Self::Data(Box::new(state))
    }
}

impl<N> TryFrom<&SimulationState<N>> for OutData
where
    N: crate::node::Node,
    N::State: Serialize,
{
    type Error = anyhow::Error;

    fn try_from(state: &crate::warding::SimulationState<N>) -> Result<Self, Self::Error> {
        todo!()
        // serde_json::to_value(state.nodes.read().iter().map(N::state).collect::<Vec<_>>())
        //     .map(OutData::new)
        //     .map_err(From::from)
    }
}

impl ToSeries for OutData {
    fn to_series(&self) -> Series {
        let s = match self {
            OutData::Runtime(runtime) => {
                let start = runtime.start.timestamp();
                let end = runtime.end.timestamp();
                let elapsed = runtime.elapsed.as_secs();
                format!("start: {}, end: {}, elapsed: {}", start, end, elapsed)
            }
            OutData::Settings(settings) => serde_json::to_string(settings).unwrap(),
            OutData::Data(state) => serde_json::to_string(state).unwrap(),
        };
        Series::new("OutData", vec![s])
    }
}
