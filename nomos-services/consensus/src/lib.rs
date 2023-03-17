//! In this module, and children ones, the 'view lifetime is tied to a logical consensus view,
//! represented by the `View` struct.
//! This is done to ensure that all the different data structs used to represent various actors
//! are always synchronized (i.e. it cannot happen that we accidentally use committees from different views).
//! It's obviously extremely important that the information contained in `View` is synchronized across different
//! nodes, but that has to be achieved through different means.
mod leadership;
mod network;
pub mod overlay;
mod tip;

// std
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;
use nomos_mempool::MempoolMsg;
// crates
use serde::{Deserialize, Serialize};
// internal
use crate::network::NetworkAdapter;
use consensus_engine::{ConsensusEngine, Input, Output, Qc, StandardQc};
use leadership::{Leadership, LeadershipResult};
use nomos_core::block::{Block, TxHash};
use nomos_core::crypto::PublicKey;
use nomos_core::fountain::FountainCode;
use nomos_core::staking::Stake;
use nomos_core::vote::Tally;
use nomos_mempool::{backend::MemPool, network::NetworkAdapter as MempoolAdapter, MempoolService};
use nomos_network::NetworkService;
use overlay::Overlay;
use overwatch_rs::services::relay::{OutboundRelay, Relay};
use overwatch_rs::services::{
    handle::ServiceStateHandle,
    relay::NoMessage,
    state::{NoOperator, NoState},
    ServiceCore, ServiceData, ServiceId,
};
use tip::Tip;

// Upper bound on the time it takes to receive a proposal for a view.
const TIMEOUT: Duration = Duration::from_secs(60);

// Raw bytes for now, could be a ed25519 public key
pub type NodeId = PublicKey;
// Random seed for each round provided by the protocol
pub type Seed = [u8; 32];

#[derive(Debug)]
pub struct CarnotSettings<Fountain: FountainCode, VoteTally: Tally> {
    private_key: [u8; 32],
    fountain_settings: Fountain::Settings,
    tally_settings: VoteTally::Settings,
}

impl<Fountain: FountainCode, VoteTally: Tally> Clone for CarnotSettings<Fountain, VoteTally> {
    fn clone(&self) -> Self {
        Self {
            private_key: self.private_key,
            fountain_settings: self.fountain_settings.clone(),
            tally_settings: self.tally_settings.clone(),
        }
    }
}

impl<Fountain: FountainCode, VoteTally: Tally> CarnotSettings<Fountain, VoteTally> {
    #[inline]
    pub const fn new(
        private_key: [u8; 32],
        fountain_settings: Fountain::Settings,
        tally_settings: VoteTally::Settings,
    ) -> Self {
        Self {
            private_key,
            fountain_settings,
            tally_settings,
        }
    }
}

pub struct CarnotConsensus<A, P, M, F, T, O>
where
    F: FountainCode,
    A: NetworkAdapter,
    M: MempoolAdapter<Tx = P::Tx>,
    P: MemPool,
    T: Tally,
    O: Overlay<A, F, T>,
    P::Tx: Debug + 'static,
    P::Id: Debug + 'static,
    A::Backend: 'static,
{
    service_state: ServiceStateHandle<Self>,
    // underlying networking backend. We need this so we can relay and check the types properly
    // when implementing ServiceCore for CarnotConsensus
    network_relay: Relay<NetworkService<A::Backend>>,
    mempool_relay: Relay<MempoolService<M, P>>,
    _fountain: std::marker::PhantomData<F>,
    _tally: std::marker::PhantomData<T>,
    _overlay: std::marker::PhantomData<O>,
}

impl<A, P, M, F, T, O> ServiceData for CarnotConsensus<A, P, M, F, T, O>
where
    F: FountainCode,
    A: NetworkAdapter,
    P: MemPool,
    T: Tally,
    P::Tx: Debug,
    P::Id: Debug,
    M: MempoolAdapter<Tx = P::Tx>,
    O: Overlay<A, F, T>,
{
    const SERVICE_ID: ServiceId = "Carnot";
    type Settings = CarnotSettings<F, T>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = NoMessage;
}

#[async_trait::async_trait]
impl<A, P, M, F, T, O> ServiceCore for CarnotConsensus<A, P, M, F, T, O>
where
    F: FountainCode + Send + Sync + 'static,
    A: NetworkAdapter + Send + Sync + 'static,
    P: MemPool + Send + Sync + 'static,
    T: Tally + Send + Sync + 'static,
    T::Settings: Send + Sync + 'static,
    T::Outcome: Send + Sync,
    P::Settings: Send + Sync + 'static,
    P::Tx: Debug + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    for<'t> &'t P::Tx: Into<TxHash>,
    P::Id: Debug
        + Clone
        + serde::de::DeserializeOwned
        + for<'a> From<&'a P::Tx>
        + Eq
        + Hash
        + Send
        + Sync
        + 'static,
    M: MempoolAdapter<Tx = P::Tx> + Send + Sync + 'static,
    O: Overlay<A, F, T, TxId = P::Id> + Send + Sync + 'static,
{
    fn init(service_state: ServiceStateHandle<Self>) -> Result<Self, overwatch_rs::DynError> {
        let network_relay = service_state.overwatch_handle.relay();
        let mempool_relay = service_state.overwatch_handle.relay();
        Ok(Self {
            service_state,
            network_relay,
            _fountain: Default::default(),
            _tally: Default::default(),
            _overlay: Default::default(),
            mempool_relay,
        })
    }

    async fn run(mut self) -> Result<(), overwatch_rs::DynError> {
        let network_relay: OutboundRelay<_> = self
            .network_relay
            .clone()
            .connect()
            .await
            .expect("Relay connection with NetworkService should succeed");

        let mempool_relay: OutboundRelay<_> = self
            .mempool_relay.clone()
            .connect()
            .await
            .expect("Relay connection with MemPoolService should succeed");

        let CarnotSettings {
            private_key,
            fountain_settings,
            tally_settings,
        } = self.service_state.settings_reader.get_updated_settings();

        let mut engine = ConsensusEngine::from_genesis(
            [0; 32],
            // FIXME: get this from the block
            Qc::Standard(StandardQc {
                view: u64::MAX,
                id: [0; 32],
            }),
        );

        let network_adapter = A::new(network_relay.clone()).await;

        let tip = Tip;
        
        // FIXME: this should be taken from config
        let mut view = View {
            seed: [0; 32],
            staking_keys: BTreeMap::new(),
            view_n: 0,
        };
        // FIXME: get the public key
        let overlay = O::new(&view, private_key);
        loop {
            // Assuming the leader also is part of a committee,
            // it will have to both propose and vote on a block in
            // different steps.
            // In addition, it could also timeout after having sent
            // a block (in the role of a committee member) if it does
            // not receive children votes in time.
            //
            // In a way, the leader task does not commit anything to
            // a node state, and the rest operates as if the node
            // is never a leader.
            let fountain = F::new(fountain_settings.clone());
            let tally = T::new(tally_settings.clone());
            let network_adapter_ = A::new(network_relay.clone()).await;
            let view_ = view.clone();
            let overlay_ = O::new(&view, private_key);
            let leadership = Leadership::<P::Tx, P::Id>::new(private_key, mempool_relay.clone());
            let tip_ = tip.clone();
            tokio::spawn(async move {
                
                let qc = overlay_.build_qc(&view_, &network_adapter_, &tally);
        
                if let LeadershipResult::Leader { block, _view } =
                    leadership.try_propose_block(&view_, &tip_, qc).await
                {
                    overlay_
                        .broadcast_block(&view_, block.clone(), &network_adapter_, &fountain)
                        .await;
                }
            });
            tokio::select! {
                _ = tokio::time::sleep(TIMEOUT) => {
                    view = view.next();
                    // do something else
                }
                block = self.happy_path(&view,  &overlay, &network_adapter, &mempool_relay, &mut engine) => {
                    // We gathered all the necessary stuff before a timeout
                    // and can now actively participate in consensus.
                    // All of the 'active' stuff happens here.
                    Self::send_my_vote(block).await;
                    view = view.next();

                }
            }
        }
    }
}

impl<A, P, M, F, T, O> CarnotConsensus<A, P, M, F, T, O>
where
    F: FountainCode + Send + Sync + 'static,
    A: NetworkAdapter + Send + Sync + 'static,
    P: MemPool + Send + Sync + 'static,
    T: Tally + Send + Sync + 'static,
    T::Settings: Send + Sync + 'static,
    T::Outcome: Send + Sync,
    P::Settings: Send + Sync + 'static,
    P::Tx: Debug + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    for<'t> &'t P::Tx: Into<TxHash>,
    P::Id: Debug
        + Clone
        + serde::de::DeserializeOwned
        + for<'a> From<&'a P::Tx>
        + Eq
        + Hash
        + Send
        + Sync
        + 'static,
    M: MempoolAdapter<Tx = P::Tx> + Send + Sync + 'static,
    O: Overlay<A, F, T, TxId = P::Id> + Send + Sync + 'static,
{
    // Gather all of the necessary stuff on the happy path to actively participate
    // in consensus.
    // Everything that happens here is non-committing, as we could timeout anytime
    // during this function.
    // However, feeding information to the consensus engine *is* committing.
    // For the time being, we will
    async fn happy_path(&self, view: &View, overlay: &O, network_adapter: &A, mempool_relay: &OutboundRelay<MempoolMsg<P::Tx, P::Id>>, engine: &mut ConsensusEngine) -> Block<P::Id> {
        let fountain = F::new(
            self.service_state
                .settings_reader
                .get_updated_settings()
                .fountain_settings,
        );       
        
        // In case this does not return when the node is a leader we might need to
        // fetch the block from the leadership service, but it's straightforward
        // to do so.
        let block = 
            // If this fails, the node has no way to recover if the network has instead decided
            // this block is part of the chain. So we try really hard to get it.
            loop {
                if let Ok(block) = overlay
                .reconstruct_proposal_block(view, network_adapter, &fountain)
                .await {
                    break block;
                }
            };
        
        // FIXME: get the qc from the block
        let outputs = engine.step(Input::Proposal{view: view.view_n, id: block.header().id(), parent_qc: Qc::Standard(StandardQc{view: view.view_n, id: block.header().id()})});
        for out in outputs {
            match out {
                Output::SafeProposal { .. } => {
                    // FIXME: what if we timeout *after* this?
                    mempool_relay
                        .send(nomos_mempool::MempoolMsg::MarkInBlock {
                            ids: block.transactions().cloned().collect(),
                            block: block.header(),
                        })
                        .await
                        .unwrap_or_else(|(e, _)| {
                            tracing::error!("Error while sending MarkInBlock message: {}", e);
                        });
                }
                Output::Committed { ..} => {
                    // FIXME: what if we timeout *after* this?
                    mempool_relay
                        .send(nomos_mempool::MempoolMsg::Prune {
                            ids: block.transactions().cloned().collect(),
                        })
                        .await
                        .unwrap_or_else(|(e, _)| {
                            tracing::error!("Error while sending MarkInBlock message: {}", e);
                        });
                }
            }
        }

        block
    }


    async fn send_my_vote(_block: Block<P::Id>) {
        todo!()
    }
}

#[derive(Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct Approval;


#[derive(Clone)]
pub struct View {
    seed: Seed,
    staking_keys: BTreeMap<NodeId, Stake>,
    pub view_n: u64,
}

impl View {
    pub fn next(&self) -> Self {
        todo!()
    }

    pub fn is_leader(&self, _node_id: NodeId) -> bool {
        true
    }
}
