use super::*;
use rand::{seq::SliceRandom, SeedableRng};

// TODO: remove

// aaah, that's a lot of nested loans, hope they don't get in the way
pub struct Member<'round, 'a, 'b, const C: usize> {
    _id: NodeId,
    _committee: &'a Committee<'round, 'b, C>,
}

pub struct Committees<'round, const C: usize> {
    _round: &'round Round,
    _view: Box<[NodeId]>,
}

pub struct Committee<'round, 'a, const C: usize> {
    _participants: Box<[NodeId]>,
    _global: &'a Committees<'round, C>,
    // could also be a const parameter, but I'm a bit wary of
    // type complexity
    _index: usize,
}

impl<'round, const C: usize> Committees<'round, C> {
    pub fn new(_round: &'round Round) -> Self {
        let mut _view = _round
            .staking_keys
            .keys()
            .cloned()
            .collect::<Box<[NodeId]>>();
        let mut rng = rand_chacha::ChaCha20Rng::from_seed(_round.seed);
        _view.shuffle(&mut rng);
        Self { _view, _round }
    }

    pub fn root_committee(&self) -> Committee<'round, '_, C> {
        todo!()
    }

    pub fn get_committee(&self, _committee_n: usize) -> Option<Committee<'round, '_, C>> {
        todo!()
    }
}

impl<'round, 'a, const C: usize> Committee<'round, 'a, C> {
    /// Return the left and right children committee, if any
    pub fn children() -> (
        Option<Committee<'round, 'a, C>>,
        Option<Committee<'round, 'a, C>>,
    ) {
        todo!()
    }

    /// Return the parent committee, if any
    pub fn parent() -> Option<Committee<'round, 'a, C>> {
        todo!()
    }
}

// TODO
// impl<'round> Index<Member> for Committee<'round> {
//     fn index() {
//         todo!()
//     }
// }

impl<'round, 'a, 'b, const C: usize> Member<'round, 'a, 'b, C> {
    /// Return other members of this committee
    pub fn peers(&self) -> &[Member<'round, 'a, 'b, C>] {
        todo!()
    }

    /// Return the participant in the parent committee this member should interact
    /// with
    pub fn parent() -> Option<Member<'round, 'a, 'b, C>> {
        todo!()
    }

    // Return participants in the children committees this member should interact with
    pub fn children() -> (
        Option<Member<'round, 'a, 'b, C>>,
        Option<Member<'round, 'a, 'b, C>>,
    ) {
        todo!()
    }
}
