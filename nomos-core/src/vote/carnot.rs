#![allow(dead_code)]
// TODO: Well, remove this when we actually use the fields from the specification
// std

use std::collections::HashSet;
// crates
use futures::{Stream, StreamExt};
// internal
use crate::block::BlockId;
use crate::crypto::PublicKey;
use crate::vote::Tally;
use consensus_engine::{AggregateQc, Qc, StandardQc, Vote};

pub type NodeId = PublicKey;

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
    type Qc = Qc;
    type Outcome = ();
    type TallyError = CarnotTallyError;
    type Settings = CarnotTallySettings;

    fn new(settings: Self::Settings) -> Self {
        Self { settings }
    }

    async fn tally<S: Stream<Item = Self::Vote> + Unpin + Send>(
        &self,
        view: consensus_engine::View,
        mut vote_stream: S,
    ) -> Result<(Self::Qc, Self::Outcome), Self::TallyError> {
        let mut approved = 0usize;
        let mut seen = HashSet::new();
        while let Some(vote) = vote_stream.next().await {
            // check vote view is valid
            if vote.view != view {
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
                return Ok((
                    Qc::Standard(StandardQc {
                        view,
                        id: vote.block,
                    }),
                    (),
                ));
            }
        }
        Err(CarnotTallyError::InsufficientVotes)
    }
}
