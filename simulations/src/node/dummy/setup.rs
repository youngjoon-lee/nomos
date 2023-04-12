// std
// crates
use crossbeam::channel;
// internal
use super::{DummyMessage, DummyNetworkInterface, DummyNode};
use crate::{
    network::Network,
    node::{NodeId, OverlayState, SharedState},
};

pub fn setup_nodes(
    node_ids: &[NodeId],
    network: &mut Network<DummyMessage>,
    overlay_state: SharedState<OverlayState>,
) -> Vec<DummyNode> {
    node_ids
        .iter()
        .map(|node_id| {
            let (node_message_sender, node_message_receiver) = channel::unbounded();
            let network_message_receiver = network.connect(*node_id, node_message_receiver);
            let network_interface =
                DummyNetworkInterface::new(*node_id, node_message_sender, network_message_receiver);
            DummyNode::new(*node_id, 0, overlay_state.clone(), network_interface)
        })
        .collect()
}
