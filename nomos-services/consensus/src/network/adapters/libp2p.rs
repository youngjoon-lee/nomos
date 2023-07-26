// std
use nomos_core::wire;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
// crates
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;
// internal
use crate::network::messages::{NewViewMsg, TimeoutMsg, TimeoutQcMsg};
use crate::network::{
    messages::{NetworkMessage, ProposalChunkMsg, VoteMsg},
    BoxedStream, NetworkAdapter,
};
use consensus_engine::{BlockId, Committee, CommitteeId, View};
use nomos_network::{
    backends::libp2p::{Command, Event, EventKind, Libp2p},
    NetworkMsg, NetworkService,
};
use overwatch_rs::services::{relay::OutboundRelay, ServiceData};

const TOPIC: &str = "/carnot/proto";
// TODO: this could be tailored per message (e.g. we need to store only a few proposals per view but might need a lot of votes)
const BUFFER_SIZE: usize = 500;

/// Due to network effects, latencies, or other factors, it is possible that a node may receive messages
/// out of order, or simply messages that are relevant to future views.
/// Since the implementation only starts listening for a message when it is needed, we need to store
/// messages so that they can be returned when needed.
///
/// Synched nodes can't fall more than a view behind the leader, and in a healthy network we expect the difference
/// between a node's view and the leader's view to be small. Given this, we can limit the size of the cache to a few
/// views and automatically clear it when the node's view is updated.
/// Messages that fall out of the cache (either evicted or never inserted because of view limits) will be discarded and
/// will have to be requested again from the network.
#[derive(Clone)]
struct MessageCache {
    // This will always contain VIEW_SIZE_LIMIT consecutive entries
    cache: Arc<Mutex<BTreeMap<View, Messages>>>,
}

// This is essentially a synchronization for a single consumer/single producer where the producer must be able to
// buffer messages even if no consumer showed up yet.
// Lock-free thread safe ring buffer exists but haven't found a good implementation for rust yet so let's just use
// channels for now.
struct Spsc<T> {
    sender: Sender<T>,
    receiver: Option<Receiver<T>>,
}

impl<T> Default for Spsc<T> {
    fn default() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(BUFFER_SIZE);
        Self {
            sender,
            receiver: Some(receiver),
        }
    }
}

#[derive(Default)]
struct Messages {
    proposal_chunks: Spsc<ProposalChunkMsg>,
    votes: HashMap<CommitteeId, HashMap<BlockId, Spsc<VoteMsg>>>,
    new_views: HashMap<CommitteeId, Spsc<NewViewMsg>>,
    timeouts: HashMap<CommitteeId, Spsc<TimeoutMsg>>,
    timeout_qcs: Spsc<TimeoutQcMsg>,
}

/// Only the first per-type stream returned by this implementation will actually contain any messages.
/// It's required for the consumer to keep the stream around for the time it's necessary
pub struct Libp2pAdapter {
    network_relay: OutboundRelay<<NetworkService<Libp2p> as ServiceData>::Message>,
    message_cache: MessageCache,
}

impl MessageCache {
    /// The number of views a node will cache messages for, from current_view to current_view + VIEW_SIZE_LIMIT.
    /// Messages for views outside [current_view, current_view + VIEW_SIZE_LIMIT will be discarded.
    const VIEW_SIZE_LIMIT: View = 5;

    // treat view as the current view
    fn advance(mut cache: impl DerefMut<Target = BTreeMap<View, Messages>>, view: View) {
        if cache.remove(&(view - 1)).is_some() {
            cache.insert(view + Self::VIEW_SIZE_LIMIT, Messages::default());
        }
    }

    // This will also advance the cache to use view - 1 as the current view
    fn get_proposals(&self, view: View) -> Option<Receiver<ProposalChunkMsg>> {
        let mut cache = self.cache.lock().unwrap();
        let res = cache
            .get_mut(&view)
            .and_then(|m| m.proposal_chunks.receiver.take());
        Self::advance(cache, view - 1);
        res
    }

    // This will also advance the cache to use view as the current view
    fn get_timeout_qcs(&self, view: View) -> Option<Receiver<TimeoutQcMsg>> {
        let mut cache = self.cache.lock().unwrap();
        let res = cache
            .get_mut(&view)
            .and_then(|m| m.timeout_qcs.receiver.take());
        Self::advance(cache, view);
        res
    }

    fn get_votes(
        &self,
        view: View,
        committee_id: CommitteeId,
        proposal_id: BlockId,
    ) -> Option<Receiver<VoteMsg>> {
        self.cache.lock().unwrap().get_mut(&view).and_then(|m| {
            m.votes.get_mut(&committee_id).and_then(|spscs| {
                spscs
                    .get_mut(&proposal_id)
                    .and_then(|spsc| spsc.receiver.take())
            })
        })
    }

    fn get_new_views(&self, view: View, committee_id: CommitteeId) -> Option<Receiver<NewViewMsg>> {
        self.cache.lock().unwrap().get_mut(&view).and_then(|m| {
            m.new_views
                .get_mut(&committee_id)
                .and_then(|spsc| spsc.receiver.take())
        })
    }

    fn get_timeouts(&self, view: View, committee_id: CommitteeId) -> Option<Receiver<TimeoutMsg>> {
        self.cache.lock().unwrap().get_mut(&view).and_then(|m| {
            m.timeouts
                .get_mut(&committee_id)
                .and_then(|spsc| spsc.receiver.take())
        })
    }
}

/// A message published via libp2p gossipsub.
/// If `to` is [`None`], it means that the `message` is propagated to all committees.
#[derive(Serialize, Deserialize)]
struct GossipsubMessage {
    to: Option<CommitteeId>,
    message: NetworkMessage,
}

impl GossipsubMessage {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
}

impl Libp2pAdapter {
    async fn broadcast(&self, message: GossipsubMessage, topic: &str) {
        if let Err((e, message)) = self
            .network_relay
            .send(NetworkMsg::Process(Command::Broadcast {
                message: message.as_bytes(),
                topic: topic.into(),
            }))
            .await
        {
            tracing::error!("error broadcasting {message:?}: {e}");
        };
    }
}

#[async_trait::async_trait]
impl NetworkAdapter for Libp2pAdapter {
    type Backend = Libp2p;

    async fn new(
        network_relay: OutboundRelay<<NetworkService<Self::Backend> as ServiceData>::Message>,
    ) -> Self {
        let message_cache = MessageCache {
            cache: Default::default(),
        };
        let cache = message_cache.clone();
        let relay = network_relay.clone();
        // TODO: maybe we need the runtime handle here?
        tokio::spawn(async move {
            // TODO: subscribe to some topic
            let (sender, receiver) = tokio::sync::oneshot::channel();
            if let Err((e, _)) = relay
                .send(NetworkMsg::Subscribe {
                    kind: EventKind::Message,
                    sender,
                })
                .await
            {
                tracing::error!("error subscribing to incoming messages: {e}");
            }
            let mut incoming_messages = receiver.await.unwrap();
            while let Ok(event) = incoming_messages.recv().await {
                match event {
                    Event::Message(message) => match nomos_core::wire::deserialize(&message.data) {
                        Ok(GossipsubMessage { to, message }) => match message {
                            NetworkMessage::ProposalChunk(msg) => {
                                tracing::debug!("received proposal chunk");
                                let mut cache = cache.cache.lock().unwrap();
                                let view = cache.keys().min().copied().unwrap();
                                if let Some(messages) = cache.get_mut(&view) {
                                    messages.proposal_chunks.sender.try_send(msg).unwrap();
                                }
                            }
                            NetworkMessage::Vote(msg) => {
                                tracing::debug!("received vote");
                                let mut cache = cache.cache.lock().unwrap();
                                let view = cache.keys().min().copied().unwrap();
                                if let Some(messages) = cache.get_mut(&view) {
                                    messages
                                        .votes
                                        .entry(to.unwrap())
                                        .or_default()
                                        .entry(msg.vote.block)
                                        .or_default()
                                        .sender
                                        .try_send(msg)
                                        .unwrap();
                                }
                            }
                            _ => tracing::debug!("unrecognized message"),
                        },
                        _ => tracing::debug!("unrecognized gossipsub message"),
                    },
                }
            }
        });
        Self {
            network_relay,
            message_cache,
        }
    }

    async fn proposal_chunks_stream(&self, view: View) -> BoxedStream<ProposalChunkMsg> {
        self.message_cache
            .get_proposals(view)
            .map::<BoxedStream<ProposalChunkMsg>, _>(|stream| Box::new(ReceiverStream::new(stream)))
            .unwrap_or_else(|| Box::new(tokio_stream::empty()))
    }

    async fn broadcast(&self, message: NetworkMessage) {
        let message = GossipsubMessage { to: None, message };
        self.broadcast(message, TOPIC).await;
    }

    async fn timeout_stream(&self, committee: &Committee, view: View) -> BoxedStream<TimeoutMsg> {
        self.message_cache
            .get_timeouts(view, committee.id::<blake2::Blake2s256>())
            .map::<BoxedStream<TimeoutMsg>, _>(|stream| Box::new(ReceiverStream::new(stream)))
            .unwrap_or_else(|| Box::new(tokio_stream::empty()))
    }

    async fn timeout_qc_stream(&self, view: View) -> BoxedStream<TimeoutQcMsg> {
        self.message_cache
            .get_timeout_qcs(view)
            .map::<BoxedStream<TimeoutQcMsg>, _>(|stream| Box::new(ReceiverStream::new(stream)))
            .unwrap_or_else(|| Box::new(tokio_stream::empty()))
    }

    async fn votes_stream(
        &self,
        committee: &Committee,
        view: View,
        proposal_id: BlockId,
    ) -> BoxedStream<VoteMsg> {
        self.message_cache
            .get_votes(view, committee.id::<blake2::Blake2s256>(), proposal_id)
            .map::<BoxedStream<VoteMsg>, _>(|stream| Box::new(ReceiverStream::new(stream)))
            .unwrap_or_else(|| Box::new(tokio_stream::empty()))
    }

    async fn new_view_stream(&self, committee: &Committee, view: View) -> BoxedStream<NewViewMsg> {
        self.message_cache
            .get_new_views(view, committee.id::<blake2::Blake2s256>())
            .map::<BoxedStream<NewViewMsg>, _>(|stream| Box::new(ReceiverStream::new(stream)))
            .unwrap_or_else(|| Box::new(tokio_stream::empty()))
    }

    async fn send(&self, message: NetworkMessage, committee: &Committee) {
        let message = GossipsubMessage {
            to: Some(committee.id::<blake2::Blake2s256>()),
            message,
        };
        self.broadcast(message, TOPIC).await;
    }
}
