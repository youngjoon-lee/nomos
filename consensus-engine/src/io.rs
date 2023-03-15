use crate::{Id, View};

/// Possible events to which the consensus engine can react.
/// Timeout is here so that the engine can abstract over time
/// and remain a pure function.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Input {
    Proposal { view: View, id: Id, parent_qc: Qc },
    NewView { view: View },
    Timeout,
}

/// Possible output events.

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Output {
    /// Proposal is "safe", meaning it's not for an old
    /// view and the qc is from the view previous to the
    /// one in which the proposal was issued.
    ///
    /// This can be used to filter out proposals for which
    /// a participant should actively vote for.
    SafeProposal {
        view: View,
        id: Id,
    },
    Committed {
        view: View,
        id: Id,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Copy)]
pub struct StandardQc {
    pub view: View,
    pub id: Id,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Copy)]
pub struct AggregatedQc {
    // FIXME: is this ever used?
    pub view: View,
    pub high_qc: StandardQc,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Copy)]
pub enum Qc {
    Standard(StandardQc),
    Aggregated(AggregatedQc),
}

impl Qc {
    /// The view of the block this qc is for.
    pub fn view(&self) -> View {
        match self {
            Qc::Standard(StandardQc { view, .. }) => *view,
            // FIXME: what about aggQc.view?
            Qc::Aggregated(AggregatedQc { high_qc, .. }) => high_qc.view,
        }
    }

    /// The id of the block this qc is for.
    /// This will be the parent of the block which will include this qc
    pub fn block(&self) -> Id {
        match self {
            Qc::Standard(StandardQc { id, .. }) => *id,
            Qc::Aggregated(AggregatedQc { high_qc, .. }) => high_qc.id,
        }
    }
}
