#![allow(dead_code)]
// TODO: Well, remove this when we actually use the fields from the specification
// std

use std::collections::HashSet;
use std::marker::PhantomData;
// crates
use futures::{Stream, StreamExt};
// internal
use crate::block::BlockId;
use crate::crypto::PublicKey;
use crate::vote::Tally;
use consensus_engine::View;

pub type NodeId = PublicKey;

impl QuorumCertificate {
    pub fn view(&self) -> View {
        match self {
            QuorumCertificate::Simple { qc, .. } => qc.view,
            QuorumCertificate::Aggregated { aggregated_qc, .. } => aggregated_qc.high_qc().view,
        }
    }
}

pub enum QuorumCertificate {
    Simple {
        qc: SimpleQuorumCertificate,
        _some_crypto_stuff: PhantomData<()>,
    },
    Aggregated {
        aggregated_qc: AggregatedQuorumCertificate,
        _some_crypto_stuff: PhantomData<()>,
    },
}

pub struct SimpleQuorumCertificate {
    view: View,
    block: BlockId,
}

pub struct AggregatedQuorumCertificate {
    view: View,
    aggregated_qcs: Box<[SimpleQuorumCertificate]>,
}

impl AggregatedQuorumCertificate {
    pub fn high_qc(&self) -> &SimpleQuorumCertificate {
        self.aggregated_qcs
            .iter()
            .max_by_key(|cert| cert.view)
            .expect("Aggregated certificates shouldn't come empty")
    }
}

pub struct Vote {
    block: BlockId,
    view: u64,
    voter: NodeId, // TODO: this should be some id, probably the node pk
    qc: Option<QuorumCertificate>,
}

impl Vote {
    pub fn valid_view(&self, view: View) -> bool {
        self.view == view && self.qc.as_ref().map_or(true, |qc| qc.view() == view - 1)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum CarnotTallyError {
    #[error("Received invalid vote: {0}")]
    InvalidVote(String),
    #[error("Did not receive enough votes")]
    InsufficientVotes,
}

#[derive(Clone)]
pub struct CarnotTallySettings {
    threshold: usize,
    // TODO: this probably should be dynamic and should change with the view (?)
    participating_nodes: HashSet<NodeId>,
}

pub struct CarnotTally {
    settings: CarnotTallySettings,
}

#[async_trait::async_trait]
impl Tally for CarnotTally {
    type Vote = Vote;
    type Outcome = QuorumCertificate;
    type TallyError = CarnotTallyError;
    type Settings = CarnotTallySettings;

    fn new(settings: Self::Settings) -> Self {
        Self { settings }
    }

    async fn tally<S: Stream<Item = Self::Vote> + Unpin + Send>(
        &self,
        view: u64,
        mut vote_stream: S,
    ) -> Result<Self::Outcome, Self::TallyError> {
        let mut approved = 0usize;
        let mut seen = HashSet::new();
        while let Some(vote) = vote_stream.next().await {
            // check vote view is valid
            if !vote.valid_view(view) {
                return Err(CarnotTallyError::InvalidVote("Invalid view".to_string()));
            }
            // check for duplicated votes
            if seen.contains(&vote.voter) {
                return Err(CarnotTallyError::InvalidVote(
                    "Double voted node".to_string(),
                ));
            }
            // check for individual nodes votes
            if !self.settings.participating_nodes.contains(&vote.voter) {
                return Err(CarnotTallyError::InvalidVote(
                    "Non-participating node".to_string(),
                ));
            }
            seen.insert(vote.voter);
            approved += 1;
            if approved >= self.settings.threshold {
                return Ok(QuorumCertificate::Simple {
                    qc: SimpleQuorumCertificate {
                        view,
                        block: vote.block,
                    },
                    _some_crypto_stuff: Default::default(),
                });
            }
        }
        Err(CarnotTallyError::InsufficientVotes)
    }
}
