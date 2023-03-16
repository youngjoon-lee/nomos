pub mod committees;
mod flat;

// std
use std::error::Error;
// crates
// internal
use super::{Approval, NodeId, View};
use crate::network::NetworkAdapter;
pub use committees::Member;
use nomos_core::block::Block;
use nomos_core::fountain::{FountainCode, FountainError};
use nomos_core::vote::Tally;

/// Dissemination overlay, tied to a specific view
#[async_trait::async_trait]
pub trait Overlay<Network: NetworkAdapter, Fountain: FountainCode, VoteTally: Tally> {
    type TxId: serde::de::DeserializeOwned + Clone + Eq + core::hash::Hash + Send + Sync + 'static;

    fn new(view: &View, node: NodeId) -> Self;

    // reconstruct proposal block from current view proposal channel
    async fn reconstruct_proposal_block(
        &self,
        view: &View,
        adapter: &Network,
        fountain: &Fountain,
    ) -> Result<Block<Self::TxId>, FountainError>;
    // broadcast a block into the current view proposal channel
    async fn broadcast_block(
        &self,
        view: &View,
        block: Block<Self::TxId>,
        adapter: &Network,
        fountain: &Fountain,
    );
    /// Wait for child votes on a block and build the certificate
    async fn build_qc(
        &self,
        view: &View,
        adapter: &Network,
        vote_tally: &VoteTally,
    ) -> VoteTally::Outcome;

    /// Forward a vote for the specified view and block
    async fn forward_vote(
        &self,
        view: &View,
        vote: VoteTally::Vote,
        adapter: &Network,
        next_view: &View,
    ) -> Result<(), Box<dyn Error>>;
}
