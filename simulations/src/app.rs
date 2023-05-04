// std
use anyhow::Ok;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
// crates
use crate::network::behaviour::create_behaviours;
use crate::network::regions::{create_regions, RegionsData};
use crate::network::{InMemoryNetworkInterface, Network};
use crate::node::carnot::CarnotNode;
use crate::node::dummy::{DummyNode, Vote};
use crate::node::{Node, NodeId, OverlayState, ViewOverlay};
use crate::output_processors::OutData;
use crate::overlay::{create_overlay, Overlay, SimulationOverlay};
use crate::runner::SimulationRunner;
use crate::settings::{self, SimulationSettings};
use crate::streaming::io::IOSubscriber;
use crate::streaming::naive::NaiveSubscriber;
use crate::streaming::polars::PolarsSubscriber;
use crate::streaming::StreamType;
use crossbeam::channel;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

/// Main simulation wrapper
pub struct SimulationApp {}

impl SimulationApp {
    pub fn run(
        simulation_settings: SimulationSettings,
        stream_type: StreamType,
    ) -> anyhow::Result<()> {
        let seed = simulation_settings.seed.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs()
        });
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut node_ids: Vec<NodeId> = (0..simulation_settings.node_count)
            .map(Into::into)
            .collect();
        node_ids.shuffle(&mut rng);

        let regions = create_regions(&node_ids, &mut rng, &simulation_settings.network_settings);
        let behaviours = create_behaviours(&simulation_settings.network_settings);
        let regions_data = RegionsData::new(regions, behaviours);
        let overlay = create_overlay(&simulation_settings.overlay_settings);
        let overlays = generate_overlays(
            &node_ids,
            &overlay,
            simulation_settings.views_count,
            simulation_settings.leaders_count,
            &mut rng,
        );

        let overlay_state = Arc::new(RwLock::new(OverlayState {
            all_nodes: node_ids.clone(),
            overlay,
            overlays: overlays.clone(),
        }));

        let mut network = Network::new(regions_data);

        match &simulation_settings.node_settings {
            settings::NodeSettings::Carnot => {
                let nodes = node_ids
                    .iter()
                    .map(|node_id| CarnotNode::new(*node_id))
                    .collect();
                run(network, nodes, simulation_settings, stream_type)?;
            }
            settings::NodeSettings::Dummy => {
                let nodes: HashMap<NodeId, DummyNode> = node_ids
                    .iter()
                    .map(|node_id| {
                        let (node_message_sender, node_message_receiver) = channel::unbounded();
                        let network_message_receiver =
                            network.connect(*node_id, node_message_receiver);
                        let network_interface = InMemoryNetworkInterface::new(
                            *node_id,
                            node_message_sender,
                            network_message_receiver,
                        );
                        (
                            *node_id,
                            DummyNode::new(*node_id, 0, overlay_state.clone(), network_interface),
                        )
                    })
                    .collect();

                // Next view leaders.
                let leaders = &overlays
                    .get(&1)
                    .ok_or_else(|| anyhow::Error::msg("no leaders"))?
                    .leaders;

                // Set initial messages from root nodes to next view leaders.
                overlays
                    .get(&0)
                    .ok_or_else(|| anyhow::Error::msg("no roots"))?
                    .layout
                    .committee_nodes(0.into())
                    .nodes
                    .iter()
                    .for_each(|r_id| {
                        leaders.iter().for_each(|l_id| {
                            nodes[r_id].send_message(*l_id, Vote::root_to_leader(1).into())
                        });
                    });

                run(
                    network,
                    Vec::from_iter(nodes.values().cloned()),
                    simulation_settings,
                    stream_type,
                )?;
            }
        };
        Ok(())
    }
}

fn run<M, N: Node>(
    network: Network<M>,
    nodes: Vec<N>,
    settings: SimulationSettings,
    stream_type: StreamType,
) -> anyhow::Result<()>
where
    M: Clone + Send + Sync + 'static,
    N: Send + Sync + 'static,
    N::Settings: Clone + Send,
    N::State: Serialize,
{
    let stream_settings = settings.stream_settings.clone();
    let runner = SimulationRunner::<_, _, OutData>::new(network, nodes, settings);
    let p = Default::default();
    macro_rules! bail {
        ($producer: ident, $settings: ident, $sub: ident) => {
            let handle = runner.simulate($producer)?;
            let mut sub_handle = handle.subscribe::<$sub<OutData>>($settings)?;
            std::thread::spawn(move || {
                sub_handle.run();
            });
            handle.join()?;
        };
    }
    match stream_type {
        StreamType::Naive => {
            let settings = stream_settings.unwrap_naive();
            bail!(p, settings, NaiveSubscriber);
        }
        StreamType::IO => {
            let settings = stream_settings.unwrap_io();
            bail!(p, settings, IOSubscriber);
        }
        StreamType::Polars => {
            let settings = stream_settings.unwrap_polars();
            bail!(p, settings, PolarsSubscriber);
        }
    };
    Ok(())
}

// Helper method to pregenerate views.
// TODO: Remove once shared overlay can generate new views on demand.
fn generate_overlays<R: Rng>(
    node_ids: &[NodeId],
    overlay: &SimulationOverlay,
    overlay_count: usize,
    leader_count: usize,
    rng: &mut R,
) -> BTreeMap<usize, ViewOverlay> {
    (0..overlay_count)
        .map(|view_id| {
            (
                view_id,
                ViewOverlay {
                    leaders: overlay.leaders(node_ids, leader_count, rng).collect(),
                    layout: overlay.layout(node_ids, rng),
                },
            )
        })
        .collect()
}
