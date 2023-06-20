use super::{Receivers, StreamSettings};
use crate::output_processors::{RecordType, Runtime};
use parking_lot::Mutex;
use polars::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Cursor,
    mem,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub enum PolarsFormat {
    Json,
    Csv,
    Parquet,
}

impl FromStr for PolarsFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "parquet" => Ok(Self::Parquet),
            tag => Err(format!(
                "Invalid {tag} format, only [json, csv, parquet] are supported",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for PolarsFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PolarsFormat::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolarsSettings {
    // pub dump_interval: std::time::Duration,
    pub format: PolarsFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl TryFrom<StreamSettings> for PolarsSettings {
    type Error = String;

    fn try_from(settings: StreamSettings) -> Result<Self, Self::Error> {
        match settings {
            StreamSettings::Polars(settings) => Ok(settings),
            _ => Err("polars settings can't be created".into()),
        }
    }
}

#[derive(Debug)]
pub struct PolarsSubscriber<R> {
    data: Arc<Mutex<Vec<Arc<R>>>>,
    path: PathBuf,
    format: PolarsFormat,
    recvs: Arc<Receivers<R>>,
    dump_interval: std::time::Duration,
}

impl<R> PolarsSubscriber<R>
where
    R: ToSeries,
{
    fn persist(&self) -> anyhow::Result<()> {
        let mut data = self.data.lock();
        let dump_data = std::mem::take(&mut *data);

        let mut data = dump_data.iter().map(|r| r.to_series()).collect();

        match self.format {
            PolarsFormat::Json => dump_dataframe_to_json(&mut data, self.path.as_path()),
            PolarsFormat::Csv => dump_dataframe_to_csv(&mut data, self.path.as_path()),
            PolarsFormat::Parquet => dump_dataframe_to_parquet(&mut data, self.path.as_path()),
        }
    }
}

impl<R> super::Subscriber for PolarsSubscriber<R>
where
    R: crate::output_processors::Record + Serialize + ToSeries,
{
    type Record = R;
    type Settings = PolarsSettings;

    fn new(
        record_recv: crossbeam::channel::Receiver<Arc<Self::Record>>,
        stop_recv: crossbeam::channel::Receiver<()>,
        settings: Self::Settings,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let recvs = Receivers {
            stop_rx: stop_recv,
            recv: record_recv,
        };
        let this = PolarsSubscriber {
            data: Arc::new(Mutex::new(Vec::new())),
            recvs: Arc::new(recvs),
            path: settings.path.clone().unwrap_or_else(|| {
                let mut p = std::env::temp_dir().join("polars");
                match settings.format {
                    PolarsFormat::Json => p.set_extension("json"),
                    PolarsFormat::Csv => p.set_extension("csv"),
                    PolarsFormat::Parquet => p.set_extension("parquet"),
                };
                p
            }),
            format: settings.format,
            // dump_interval: settings.dump_interval,
            dump_interval: std::time::Duration::from_secs(1),
        };
        tracing::info!(
            target = "simulation",
            "subscribed to {}",
            this.path.display()
        );
        Ok(this)
    }

    fn next(&self) -> Option<anyhow::Result<Arc<Self::Record>>> {
        Some(self.recvs.recv.recv().map_err(From::from))
    }

    fn run(self) -> anyhow::Result<()> {
        let ticker = crossbeam::channel::tick(self.dump_interval);
        let mut file_idx = 0;
        loop {
            crossbeam::select! {
                recv(self.recvs.stop_rx) -> _ => {
                    // collect the run time meta
                    self.sink(Arc::new(R::from(Runtime::load()?)))?;
                    return self.persist();
                }
                recv(self.recvs.recv) -> msg => {
                    self.sink(msg?)?;
                }
                recv(ticker) -> _ => {
                    match self.persist() {
                        Ok(_) => {
                            tracing::info!(
                                target = "simulation",
                                "persisted intermediary",
                            );
                            file_idx += 1;
                        },
                        Err(e) => tracing::error!(target = "simulation", "persist error: {}", e),
                    }
                }
            }
        }
    }

    fn sink(&self, state: Arc<Self::Record>) -> anyhow::Result<()> {
        self.data.lock().push(state);
        Ok(())
    }

    fn subscribe_data_type() -> RecordType {
        RecordType::Data
    }
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

pub trait ToSeries {
    fn to_series(&self) -> Series;
}
