use std::{
    collections::{BTreeMap, HashSet},
    panic,
};

use consensus_engine::{
    AggregateQc, Block, LeaderProof, NewView, NodeId, Qc, StandardQc, TimeoutQc, View,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::{prop_state_machine, ReferenceStateMachine, StateMachineTest};
use system_under_test::ConsensusEngineTest;

prop_state_machine! {
    #![proptest_config(Config {
        // Only run 100 cases by default to avoid running out of system resources
        // and taking too long to finish.
        cases: 100,
        .. Config::default()
    })]

    #[test]
    // run 50 state transitions per test case
    fn happy_path(sequential 1..50 => ConsensusEngineTest);
}

// A reference state machine (RefState) is used to generated state transitions.
// To generate some kinds of transition, we may need to keep historical blocks in RefState.
// Also, RefState can be used to check invariants of the real state machine in some cases.
//
// We should try to design this reference state as simple/intuitive as possible,
// so that we don't need to replicate the logic implemented in consensus-engine.
#[derive(Clone, Debug)]
pub struct RefState {
    chain: BTreeMap<View, ViewEntry>,
    highest_voted_view: View,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ViewEntry {
    blocks: HashSet<Block>,
    timeout_qcs: HashSet<TimeoutQc>,
}

impl ViewEntry {
    fn new() -> ViewEntry {
        Default::default()
    }

    fn is_new_view(&self) -> bool {
        self.blocks.is_empty() && self.timeout_qcs.is_empty()
    }

    fn high_qc(&self) -> Option<StandardQc> {
        let iter1 = self.blocks.iter().map(|block| block.parent_qc.high_qc());
        let iter2 = self
            .timeout_qcs
            .iter()
            .map(|timeout_qc| timeout_qc.high_qc.clone());
        iter1.chain(iter2).max_by_key(|qc| qc.view)
    }
}

// State transtitions that will be picked randomly
#[derive(Clone, Debug)]
pub enum Transition {
    Nop,
    ReceiveBlock(Block),
    ReceiveUnsafeBlock(Block),
    ApproveBlock(Block),
    ApprovePastBlock(Block),
    LocalTimeout,
    ReceiveTimeoutQcForCurrentView(TimeoutQc),
    ReceiveTimeoutQcForOldView(TimeoutQc),
    ApproveNewViewWithLatestTimeoutQc(TimeoutQc, HashSet<NewView>),
    //TODO: add more invalid transitions that must be rejected by consensus-engine
}

const LEADER_PROOF: LeaderProof = LeaderProof::LeaderId { leader_id: [0; 32] };
const INITIAL_HIGHEST_VOTED_VIEW: View = -1;
const SENDER: NodeId = [0; 32];

impl ReferenceStateMachine for RefState {
    type State = Self;

    type Transition = Transition;

    // Initialize the reference state machine
    fn init_state() -> BoxedStrategy<Self::State> {
        let genesis_block = Block {
            view: 0,
            id: [0; 32],
            parent_qc: Qc::Standard(StandardQc::genesis()),
            leader_proof: LEADER_PROOF.clone(),
        };

        Just(RefState {
            chain: BTreeMap::from([(
                genesis_block.view,
                ViewEntry {
                    blocks: HashSet::from([genesis_block]),
                    timeout_qcs: Default::default(),
                },
            )]),
            highest_voted_view: INITIAL_HIGHEST_VOTED_VIEW,
        })
        .boxed()
    }

    // Generate transitions based on the current reference state machine
    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        // Instead of using verbose `if` statements here to filter out the types of transitions
        // which cannot be created based on the current reference state,
        // each `state.transition_*` function returns a Nop transition
        // if it cannot generate the promised transition for the current reference state.
        // Both reference and real state machine do nothing for Nop transitions.
        prop_oneof![
            state.transition_receive_block(),
            state.transition_receive_unsafe_block(),
            state.transition_approve_block(),
            state.transition_approve_past_block(),
            state.transition_local_timeout(),
            state.transition_receive_timeout_qc_for_current_view(),
            state.transition_receive_timeout_qc_for_old_view(),
            state.transition_approve_new_view_with_latest_timeout_qc(),
            state.transition_receive_block_with_aggregated_qc(),
        ]
        .boxed()
    }

    // Check if the transition is valid for a given reference state, before applying the transition
    // If invalid, the transition will be ignored and a new transition will be generated.
    //
    // Also, preconditions are used for shrinking in failure cases.
    // Preconditions check if the transition is still valid after some shrinking is applied.
    // If the transition became invalid for the shrinked state, the shrinking is stopped or
    // is continued to other directions.
    fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool {
        // In most cases, we need to check the same conditions again used to create transitions.
        // This is redundant for success cases, but is necessary for shrinking in failure cases,
        // because some transitions may no longer be valid after some shrinking is applied.
        match transition {
            Transition::Nop => true,
            Transition::ReceiveBlock(block) => block.view >= state.current_view(),
            Transition::ReceiveUnsafeBlock(block) => block.view < state.current_view(),
            Transition::ApproveBlock(block) => state.highest_voted_view < block.view,
            Transition::ApprovePastBlock(block) => state.highest_voted_view >= block.view,
            Transition::LocalTimeout => true,
            Transition::ReceiveTimeoutQcForCurrentView(timeout_qc) => {
                timeout_qc.view == state.current_view()
            }
            Transition::ReceiveTimeoutQcForOldView(timeout_qc) => {
                timeout_qc.view < state.current_view()
            }
            Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, _) => {
                let is_latest_timeout_qc = state
                    .chain
                    .last_key_value()
                    .unwrap()
                    .1
                    .timeout_qcs
                    .contains(timeout_qc);

                is_latest_timeout_qc
                    && state.highest_voted_view < RefState::new_view_from(timeout_qc)
            }
        }
    }

    // Apply the given transition on the reference state machine,
    // so that it can be used to generate next transitions.
    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Nop => {}
            Transition::ReceiveBlock(block) => {
                state
                    .chain
                    .entry(block.view)
                    .or_default()
                    .blocks
                    .insert(block.clone());
            }
            Transition::ReceiveUnsafeBlock(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::ApproveBlock(block) => {
                state.highest_voted_view = block.view;
            }
            Transition::ApprovePastBlock(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::LocalTimeout => {
                state.highest_voted_view = state.current_view();
            }
            Transition::ReceiveTimeoutQcForCurrentView(timeout_qc) => {
                state
                    .chain
                    .entry(timeout_qc.view)
                    .or_default()
                    .timeout_qcs
                    .insert(timeout_qc.clone());
            }
            Transition::ReceiveTimeoutQcForOldView(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, _) => {
                let new_view = RefState::new_view_from(timeout_qc);
                state.chain.entry(new_view).or_insert(ViewEntry::new());
                state.highest_voted_view = new_view;
            }
        }

        state
    }
}

impl RefState {
    // Generate a Transition::ReceiveBlock.
    fn transition_receive_block(&self) -> BoxedStrategy<Transition> {
        let recent_parents = self
            .chain
            .range(self.current_view() - 1..)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if recent_parents.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(recent_parents)
                .prop_map(move |parent| -> Transition {
                    Transition::ReceiveBlock(Self::consecutive_block(&parent))
                })
                .boxed()
        }
    }

    // Generate a Transition::ReceiveUnsafeBlock.
    fn transition_receive_unsafe_block(&self) -> BoxedStrategy<Transition> {
        let old_parents = self
            .chain
            .range(..self.current_view() - 1)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if old_parents.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(old_parents)
                .prop_map(move |parent| -> Transition {
                    Transition::ReceiveUnsafeBlock(Self::consecutive_block(&parent))
                })
                .boxed()
        }
    }

    // Generate a Transition::ApproveBlock.
    fn transition_approve_block(&self) -> BoxedStrategy<Transition> {
        let blocks_not_voted = self
            .chain
            .range(self.highest_voted_view + 1..)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if blocks_not_voted.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(blocks_not_voted)
                .prop_map(move |block| Transition::ApproveBlock(block))
                .boxed()
        }
    }

    // Generate a Transition::ApprovePastBlock.
    fn transition_approve_past_block(&self) -> BoxedStrategy<Transition> {
        let past_blocks = self
            .chain
            .range(INITIAL_HIGHEST_VOTED_VIEW..self.highest_voted_view)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if past_blocks.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(past_blocks)
                .prop_map(move |block| Transition::ApprovePastBlock(block))
                .boxed()
        }
    }

    // Generate a Transition::LocalTimeout.
    fn transition_local_timeout(&self) -> BoxedStrategy<Transition> {
        Just(Transition::LocalTimeout).boxed()
    }

    // Generate a Transition::ReceiveTimeoutQcForCurrentView
    fn transition_receive_timeout_qc_for_current_view(&self) -> BoxedStrategy<Transition> {
        let view = self.current_view();
        let high_qc = self
            .chain
            .iter()
            .rev()
            .find_map(|(_, entry)| entry.high_qc());

        if let Some(high_qc) = high_qc {
            Just(Transition::ReceiveTimeoutQcForCurrentView(TimeoutQc {
                view: view.clone(),
                high_qc,
                sender: SENDER.clone(),
            }))
            .boxed()
        } else {
            Just(Transition::Nop).boxed()
        }
    }

    // Generate a Transition::ReceiveTimeoutQcForOldView
    fn transition_receive_timeout_qc_for_old_view(&self) -> BoxedStrategy<Transition> {
        let old_view_entries: Vec<(View, ViewEntry)> = self
            .chain
            .range(..self.current_view())
            .filter(|(_, entry)| !entry.blocks.is_empty())
            .map(|(view, entry)| (view.clone(), entry.clone()))
            .collect();

        if old_view_entries.is_empty() {
            return Just(Transition::Nop).boxed();
        }

        proptest::sample::select(old_view_entries)
            .prop_map(move |(view, entry)| {
                Transition::ReceiveTimeoutQcForOldView(TimeoutQc {
                    view: view.clone(),
                    high_qc: entry.high_qc().unwrap(),
                    sender: SENDER.clone(),
                })
            })
            .boxed()
    }

    // Generate a Transition::ApproveNewViewWithLatestTimeoutQc.
    fn transition_approve_new_view_with_latest_timeout_qc(&self) -> BoxedStrategy<Transition> {
        let timeout_qcs: Vec<TimeoutQc> = self
            .chain
            .last_key_value()
            .unwrap()
            .1
            .timeout_qcs
            .iter()
            .filter(|timeout_qc| self.highest_voted_view < RefState::new_view_from(timeout_qc))
            .cloned()
            .collect();

        if timeout_qcs.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            proptest::sample::select(timeout_qcs)
                .prop_map(move |timeout_qc| {
                    //TODO: set new_views
                    Transition::ApproveNewViewWithLatestTimeoutQc(
                        timeout_qc.clone(),
                        HashSet::new(),
                    )
                })
                .boxed()
        }
    }

    // Generate a Transition::ReceiveBlockWithAggregatedQc
    fn transition_receive_block_with_aggregated_qc(&self) -> BoxedStrategy<Transition> {
        //TODO: more randomness
        let (last_view, last_entry) = self.chain.last_key_value().unwrap();
        if !last_entry.is_new_view() {
            return Just(Transition::Nop).boxed();
        }
        let new_view = last_view;

        //TODO: get high_qc from NewView message
        let high_qc = self
            .chain
            .get(&(new_view - 1))
            .unwrap()
            .timeout_qcs
            .iter()
            .max_by_key(|timeout_qc| timeout_qc.high_qc.view)
            .unwrap()
            .clone()
            .high_qc;

        Just(Transition::ReceiveBlock(Block {
            id: rand::thread_rng().gen(),
            view: new_view + 1,
            parent_qc: Qc::Aggregated(AggregateQc {
                high_qc: high_qc.clone(),
                view: new_view.clone(),
            }),
            leader_proof: LEADER_PROOF.clone(),
        }))
        .boxed()
    }

    fn current_view(&self) -> View {
        let (last_view, last_entry) = self.chain.last_key_value().unwrap();
        if last_entry.timeout_qcs.is_empty() {
            last_view.clone()
        } else {
            let timeout_qc = last_entry.timeout_qcs.iter().next().unwrap();
            RefState::new_view_from(&timeout_qc)
        }
    }

    fn new_view_from(timeout_qc: &TimeoutQc) -> View {
        timeout_qc.view + 1
    }

    fn consecutive_block(parent: &Block) -> Block {
        Block {
            // use rand because we don't want this to be shrinked by proptest
            id: rand::thread_rng().gen(),
            view: parent.view + 1,
            parent_qc: Qc::Standard(StandardQc {
                view: parent.view,
                id: parent.id,
            }),
            leader_proof: LEADER_PROOF.clone(),
        }
    }
}

// StateMachineTest defines how transitions are applied to the real state machine
// and what checks should be performed.
impl StateMachineTest for ConsensusEngineTest {
    // SUT is the real state machine that we want to test.
    type SystemUnderTest = Self;

    // A reference state machine that should be compared against the SUT.
    type Reference = RefState;

    // Initialize the SUT state
    fn init_test(
        _ref_state: &<Self::Reference as proptest_state_machine::ReferenceStateMachine>::State,
    ) -> Self::SystemUnderTest {
        ConsensusEngineTest::new()
    }

    // Apply the transition on the SUT state and check post-conditions
    fn apply(
        state: Self::SystemUnderTest,
        _ref_state: &<Self::Reference as proptest_state_machine::ReferenceStateMachine>::State,
        transition: <Self::Reference as proptest_state_machine::ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        println!("{transition:?}");

        match transition {
            Transition::Nop => state,
            Transition::ReceiveBlock(block) => {
                let engine = state.engine.receive_block(block.clone()).unwrap();
                assert!(engine.blocks_in_view(block.view).contains(&block));

                ConsensusEngineTest { engine }
            }
            Transition::ReceiveUnsafeBlock(block) => {
                let result = panic::catch_unwind(|| state.engine.receive_block(block));
                assert!(result.is_err() || result.unwrap().is_err());

                state
            }
            Transition::ApproveBlock(block) => {
                let (engine, _) = state.engine.approve_block(block.clone());
                assert_eq!(engine.highest_voted_view(), block.view);

                ConsensusEngineTest { engine }
            }
            Transition::ApprovePastBlock(block) => {
                let result = panic::catch_unwind(|| state.engine.approve_block(block));
                assert!(result.is_err());

                state
            }
            Transition::LocalTimeout => {
                let (engine, _) = state.engine.local_timeout();
                assert_eq!(engine.highest_voted_view(), engine.current_view());

                ConsensusEngineTest { engine }
            }
            Transition::ReceiveTimeoutQcForCurrentView(timeout_qc) => {
                let engine = state.engine.receive_timeout_qc(timeout_qc.clone());
                assert_eq!(engine.current_view(), RefState::new_view_from(&timeout_qc));

                ConsensusEngineTest { engine }
            }
            Transition::ReceiveTimeoutQcForOldView(timeout_qc) => {
                let prev_engine = state.engine.clone();
                let engine = state.engine.receive_timeout_qc(timeout_qc.clone());

                // Check that the engine state didn't change.
                assert_eq!(
                    engine.last_view_timeout_qc(),
                    prev_engine.last_view_timeout_qc()
                );
                assert_eq!(engine.current_view(), prev_engine.current_view());

                ConsensusEngineTest { engine }
            }
            Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, new_views) => {
                let (engine, _) = state.engine.approve_new_view(timeout_qc.clone(), new_views);
                assert_eq!(
                    engine.highest_voted_view(),
                    RefState::new_view_from(&timeout_qc)
                );
                assert_eq!(engine.high_qc().view, timeout_qc.high_qc.view);

                ConsensusEngineTest { engine }
            }
        }
    }

    // Check invariants after every transition.
    // We have the option to use RefState for comparison in some cases.
    fn check_invariants(
        state: &Self::SystemUnderTest,
        ref_state: &<Self::Reference as ReferenceStateMachine>::State,
    ) {
        assert_eq!(state.engine.current_view(), ref_state.current_view());

        assert_eq!(
            state.engine.highest_voted_view(),
            ref_state.highest_voted_view
        );

        //TODO: add more invariants with more public functions of Carnot
    }
}

// SUT is a system (state) that we want to test.
mod system_under_test {
    use consensus_engine::{
        overlay::{FlatOverlay, RoundRobin, Settings},
        *,
    };

    use crate::LEADER_PROOF;

    #[derive(Clone, Debug)]
    pub struct ConsensusEngineTest {
        pub engine: Carnot<FlatOverlay<RoundRobin>>,
    }

    impl ConsensusEngineTest {
        pub fn new() -> Self {
            let engine = Carnot::from_genesis(
                [0; 32],
                Block {
                    view: 0,
                    id: [0; 32],
                    parent_qc: Qc::Standard(StandardQc::genesis()),
                    leader_proof: LEADER_PROOF.clone(),
                },
                FlatOverlay::new(Settings {
                    nodes: vec![[0; 32]],
                    leader: RoundRobin::default(),
                }),
            );

            ConsensusEngineTest { engine }
        }
    }
}
