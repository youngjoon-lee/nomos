pub mod carnot;
pub mod mock;

// std
// crates
use futures::Stream;
//internal

#[async_trait::async_trait]
pub trait Tally {
    type Vote;
    type Outcome;
    type Issuer;
    type TallyError;
    type Settings: Clone;
    fn new(settings: Self::Settings) -> Self;
    async fn tally<S: Stream<Item = Self::Vote> + Unpin + Send>(
        &self,
        view: u64,
        vote_stream: S,
    ) -> Result<Self::Outcome, Self::TallyError>;
    fn vote_from_outcome(
        outcome: Self::Outcome,
        view: u64,
        issuer: Self::Issuer,
    ) -> Result<Self::Vote, Self::TallyError>;
}
