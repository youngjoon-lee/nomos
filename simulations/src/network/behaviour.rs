// std
use std::{collections::HashMap, time::Duration};
// crates
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::{regions::Region, NetworkSettings};
// internal

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct NetworkBehaviour {
    pub delay: Duration,
    pub drop: f64,
}

impl NetworkBehaviour {
    pub fn new(delay: Duration, drop: f64) -> Self {
        Self { delay, drop }
    }

    pub fn delay(&self) -> Duration {
        self.delay
    }

    pub fn should_drop<R: Rng>(&self, rng: &mut R) -> bool {
        rng.gen_bool(self.drop)
    }
}

// Takes a reference to the network_settings and returns a HashMap representing the
// network behaviors for pairs of NodeIds.
pub fn create_behaviours(
    network_settings: &NetworkSettings,
) -> HashMap<(Region, Region), NetworkBehaviour> {
    let mut behaviours = HashMap::new();

    for nd in &network_settings.network_behaviors {
        for (i, delay) in nd.delays.iter().enumerate() {
            let from_region = nd.region;
            let to_region = match i {
                0 => Region::NorthAmerica,
                1 => Region::Europe,
                2 => Region::Asia,
                3 => Region::Africa,
                4 => Region::SouthAmerica,
                5 => Region::Australia,
                _ => panic!("Invalid region index"),
            };

            let behaviour =
                NetworkBehaviour::new(std::time::Duration::from_millis(*delay as u64), 0.0);
            behaviours.insert((from_region, to_region), behaviour);
        }
    }

    behaviours
}
