// std
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
// crates
use futures::Stream;
use nomos_core::wire;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
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

type Relay<T> = OutboundRelay<<NetworkService<T> as ServiceData>::Message>;

macro_rules! make_stream {
    ($name:ident, $field:ident, $type:ty) => {
        pub struct $name {
            cache: Arc<Mutex<BTreeMap<View, Messages>>>,
            view: View,
            to: Option<CommitteeId>,
            filter: Box<dyn Fn(&$type) -> bool + Send + Sync>,
        }

        impl Stream for $name {
            type Item = $type;

            fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                let mut cache = self.cache.lock().unwrap();
                match cache.get_mut(&self.view) {
                    Some(messages) => {
                        if let Some(buffer) = messages.$field.buffers.get_mut(&self.to) {
                            if let Some(idx) = buffer.iter().position(&self.filter) {
                                return Poll::Ready(buffer.swap_remove_front(idx));
                            }
                        }
                        messages.$field.wakers.push(cx.waker().clone());
                        drop(cache);
                        Poll::Pending
                    }
                    None => Poll::Ready(None),
                }
            }
        }
    };
}

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

struct Spsc<T> {
    buffers: HashMap<Option<CommitteeId>, VecDeque<T>>,
    wakers: Vec<Waker>,
}

impl<T> Spsc<T> {
    fn insert(&mut self, to: Option<CommitteeId>, t: T) {
        self.buffers
            .entry(to)
            .or_insert(VecDeque::with_capacity(BUFFER_SIZE))
            .push_back(t);
        for waker in self.wakers.drain(..) {
            waker.wake();
        }
    }
}

impl<T> Default for Spsc<T> {
    fn default() -> Self {
        Self {
            buffers: HashMap::new(),
            wakers: Vec::new(),
        }
    }
}

make_stream!(VotesMessageStream, votes, VoteMsg);
make_stream!(ProposalMessageStream, proposal_chunks, ProposalChunkMsg);
make_stream!(NewViewStream, new_views, NewViewMsg);
make_stream!(TimeoutStream, timeouts, TimeoutMsg);
make_stream!(TimeoutQcStream, timeout_qcs, TimeoutQcMsg);

#[derive(Default)]
struct Messages {
    proposal_chunks: Spsc<ProposalChunkMsg>,
    votes: Spsc<VoteMsg>,
    new_views: Spsc<NewViewMsg>,
    timeouts: Spsc<TimeoutMsg>,
    timeout_qcs: Spsc<TimeoutQcMsg>,
}

#[derive(Clone)]
pub struct Libp2pAdapter {
    network_relay: OutboundRelay<<NetworkService<Libp2p> as ServiceData>::Message>,
    message_cache: MessageCache,
}

impl MessageCache {
    /// The number of views a node will cache messages for, from current_view to current_view + VIEW_SIZE_LIMIT.
    /// Messages for views outside [current_view, current_view + VIEW_SIZE_LIMIT] will be discarded.
    const VIEW_SIZE_LIMIT: View = 5;

    fn new() -> Self {
        let cache = (0..Self::VIEW_SIZE_LIMIT)
            .map(|v| (v, Default::default()))
            .collect::<BTreeMap<View, Messages>>();
        Self {
            cache: Arc::new(Mutex::new(cache)),
        }
    }

    // treat view as the current view
    fn advance(mut cache: impl DerefMut<Target = BTreeMap<View, Messages>>, view: View) {
        if cache.remove(&(view - 1)).is_some() {
            cache.insert(view + Self::VIEW_SIZE_LIMIT - 1, Messages::default());
        }
    }

    // This will also advance the cache to use view - 1 as the current view
    fn get_proposals(&self, view: View) -> ProposalMessageStream {
        let cache = self.cache.lock().unwrap();
        Self::advance(cache, view - 1);
        ProposalMessageStream {
            cache: self.cache.clone(),
            view,
            to: None,
            filter: Box::new(|_: &ProposalChunkMsg| true),
        }
    }

    // This will also advance the cache to use view as the current view
    fn get_timeout_qcs(&self, view: View) -> TimeoutQcStream {
        let cache = self.cache.lock().unwrap();
        Self::advance(cache, view);
        TimeoutQcStream {
            cache: self.cache.clone(),
            view,
            to: None,
            filter: Box::new(|_: &TimeoutQcMsg| true),
        }
    }

    fn get_votes(
        &self,
        view: View,
        committee_id: CommitteeId,
        proposal_id: BlockId,
    ) -> VotesMessageStream {
        VotesMessageStream {
            cache: self.cache.clone(),
            view,
            to: Some(committee_id),
            filter: Box::new(move |msg: &VoteMsg| msg.vote.block == proposal_id),
        }
    }

    fn get_new_views(&self, view: View, committee_id: CommitteeId) -> NewViewStream {
        NewViewStream {
            cache: self.cache.clone(),
            view,
            to: Some(committee_id),
            filter: Box::new(|_: &NewViewMsg| true),
        }
    }

    fn get_timeouts(&self, view: View, committee_id: CommitteeId) -> TimeoutStream {
        TimeoutStream {
            cache: self.cache.clone(),
            view,
            to: Some(committee_id),
            filter: Box::new(|_: &TimeoutMsg| true),
        }
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

    async fn subscribe(relay: &Relay<Libp2p>, topic: &str) {
        if let Err((e, _)) = relay
            .send(NetworkMsg::Process(Command::Subscribe(topic.into())))
            .await
        {
            tracing::error!("error subscribing to {topic}: {e}");
        };
    }
}

#[async_trait::async_trait]
impl NetworkAdapter for Libp2pAdapter {
    type Backend = Libp2p;

    async fn new(network_relay: Relay<Libp2p>) -> Self {
        let message_cache = MessageCache::new();
        let cache = message_cache.clone();
        let relay = network_relay.clone();
        // TODO: maybe we need the runtime handle here?
        tokio::spawn(async move {
            Self::subscribe(&relay, TOPIC).await;
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
            loop {
                match incoming_messages.recv().await {
                    Ok(event) => match event {
                        Event::Message(message) => {
                            match nomos_core::wire::deserialize(&message.data) {
                                Ok(GossipsubMessage { to, message }) => match message {
                                    NetworkMessage::ProposalChunk(msg) => {
                                        tracing::debug!("received proposal chunk");
                                        let mut cache = cache.cache.lock().unwrap();
                                        let view = msg.view;
                                        if let Some(messages) = cache.get_mut(&view) {
                                            messages.proposal_chunks.insert(to, msg);
                                        }
                                        drop(cache);
                                    }
                                    NetworkMessage::Vote(msg) => {
                                        tracing::debug!("received vote");
                                        let mut cache = cache.cache.lock().unwrap();
                                        let view = msg.vote.view;
                                        if let Some(messages) = cache.get_mut(&view) {
                                            messages.votes.insert(to, msg);
                                        }
                                        drop(cache);
                                    }
                                    NetworkMessage::Timeout(msg) => {
                                        tracing::debug!("received timeout");
                                        let mut cache = cache.cache.lock().unwrap();
                                        let view = msg.vote.view;
                                        if let Some(messages) = cache.get_mut(&view) {
                                            messages.timeouts.insert(to, msg);
                                        }
                                        drop(cache);
                                    }
                                    NetworkMessage::TimeoutQc(msg) => {
                                        tracing::debug!("received timeout_qc");
                                        let mut cache = cache.cache.lock().unwrap();
                                        let view = msg.qc.view();
                                        if let Some(messages) = cache.get_mut(&view) {
                                            messages.timeout_qcs.insert(to, msg);
                                        }
                                        drop(cache);
                                    }
                                    NetworkMessage::NewView(msg) => {
                                        tracing::debug!("received new_view");
                                        let mut cache = cache.cache.lock().unwrap();
                                        let view = msg.vote.view;
                                        if let Some(messages) = cache.get_mut(&view) {
                                            messages.new_views.insert(to, msg);
                                        }
                                        drop(cache);
                                    }
                                },
                                _ => tracing::error!("unrecognized gossipsub message"),
                            }
                        }
                    },
                    Err(RecvError::Lagged(n)) => {
                        tracing::error!("lagged messages: {n}")
                    }
                    Err(RecvError::Closed) => unreachable!(),
                }
            }
        });
        Self {
            network_relay,
            message_cache,
        }
    }

    async fn proposal_chunks_stream(&self, view: View) -> BoxedStream<ProposalChunkMsg> {
        Box::new(self.message_cache.get_proposals(view))
    }

    async fn broadcast(&self, message: NetworkMessage) {
        let message = GossipsubMessage { to: None, message };
        self.broadcast(message, TOPIC).await;
    }

    async fn timeout_stream(&self, committee: &Committee, view: View) -> BoxedStream<TimeoutMsg> {
        Box::new(
            self.message_cache
                .get_timeouts(view, committee.id::<blake2::Blake2s256>()),
        )
    }

    async fn timeout_qc_stream(&self, view: View) -> BoxedStream<TimeoutQcMsg> {
        Box::new(self.message_cache.get_timeout_qcs(view))
    }

    async fn votes_stream(
        &self,
        committee: &Committee,
        view: View,
        proposal_id: BlockId,
    ) -> BoxedStream<VoteMsg> {
        Box::new(self.message_cache.get_votes(
            view,
            committee.id::<blake2::Blake2s256>(),
            proposal_id,
        ))
    }

    async fn new_view_stream(&self, committee: &Committee, view: View) -> BoxedStream<NewViewMsg> {
        Box::new(
            self.message_cache
                .get_new_views(view, committee.id::<blake2::Blake2s256>()),
        )
    }

    async fn send(&self, message: NetworkMessage, committee: &Committee) {
        let message = GossipsubMessage {
            to: Some(committee.id::<blake2::Blake2s256>()),
            message,
        };
        self.broadcast(message, TOPIC).await;
    }
}
