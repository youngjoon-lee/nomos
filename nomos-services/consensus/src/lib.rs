//! In this module, and children ones, the 'round lifetime is tied to a logical consensus round,
//! represented by the `Round` struct.
//! A `Round` cannot be copied or cloned, and a new one can only be created by replacing the old one.
//! This is done to ensure that all the different data structs used to represent various actors
//! are always synchronized (i.e. it cannot happen that we accidentally use committees from different rounds).
//! It's obviously extremely important that the information contained in `Round` is synchronized across different
//! nodes, but that has to be achieved through different means.
pub mod committees;

use std::collections::BTreeMap;

// Raw bytes for now, could be a ed25519 public key
pub type NodeId = [u8; 32];
// Random seed for each round provided by the protocol
pub type Seed = [u8; 32];
pub type Stake = u64;

// Consensus round, also aids in guaranteeing synchronization
// between various data structures by means of lifetimes
pub struct Round {
    staking_keys: BTreeMap<NodeId, Stake>,
    seed: Seed,
    id: u64,
}

impl Round {
    /// First round ever
    pub fn zero() -> Self {
        Round {
            staking_keys: BTreeMap::new(),
            seed: [0; 32],
            id: 0,
        }
    }

    /// Build next round
    pub fn next(self, staking_keys: BTreeMap<NodeId, Stake>, seed: Seed) -> Self {
        Self {
            staking_keys,
            seed,
            id: self.id + 1,
        }
    }
}
