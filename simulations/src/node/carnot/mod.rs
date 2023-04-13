// std
// crates
use crate::network::InMemoryNetworkInterface;
use serde::{Deserialize, Serialize};

// internal
use super::{Node, NodeId, OverlayState, SharedState};

#[derive(Default, Serialize)]
pub struct CarnotState {}

#[derive(Clone, Default, Deserialize)]
pub struct CarnotSettings {}

#[allow(dead_code)] // TODO: remove when handling settings
pub struct CarnotNode {
    id: NodeId,
    state: CarnotState,
    settings: CarnotSettings,
}

impl Node for CarnotNode {
    type Settings = CarnotSettings;
    type State = CarnotState;
    type NetworkInterface = InMemoryNetworkInterface<()>;

    fn new(
        node_id: NodeId,
        _view_id: usize,
        _overlay_state: SharedState<OverlayState>,
        _network_interface: Self::NetworkInterface,
    ) -> Self {
        Self {
            id: node_id,
            state: Default::default(),
            settings: Default::default(),
        }
    }

    fn id(&self) -> NodeId {
        self.id
    }

    fn current_view(&self) -> usize {
        todo!()
    }

    fn state(&self) -> &CarnotState {
        &self.state
    }

    fn step(&mut self) {
        todo!()
    }
}
