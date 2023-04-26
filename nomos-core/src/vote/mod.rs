pub mod carnot;
pub mod mock;

use futures::Stream;

#[async_trait::async_trait]
pub trait Tally {
    type Vote;
    type Qc;
    type Outcome;
    type TallyError;
    type Settings: Clone;
    fn new(settings: Self::Settings) -> Self;
    async fn tally<S: Stream<Item = Self::Vote> + Unpin + Send>(
        &self,
        view: consensus_engine::View,
        vote_stream: S,
    ) -> Result<(Self::Qc, Self::Outcome), Self::TallyError>;
}
