pub mod command;
pub mod event;
mod swarm;

use std::error::Error;

use command::Command;
use event::Event;
use serde::{Deserialize, Serialize};
use swarm::{Swarm, SwarmConfig};
use tokio::sync::{broadcast, mpsc};

pub struct NomosLibp2p {
    command_tx: mpsc::Sender<Command>,
    event_tx: broadcast::Sender<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NomosLibp2pConfig {
    pub swarm_config: SwarmConfig,
    pub command_channel_size: usize,
    pub event_channel_size: usize,
}

impl Default for NomosLibp2pConfig {
    fn default() -> Self {
        Self {
            swarm_config: Default::default(),
            command_channel_size: 16,
            event_channel_size: 16,
        }
    }
}

impl NomosLibp2p {
    // TODO: define error types
    pub fn new(
        config: NomosLibp2pConfig,
        runtime_handle: tokio::runtime::Handle,
    ) -> Result<Self, Box<dyn Error>> {
        let (command_tx, command_rx) = mpsc::channel::<Command>(config.command_channel_size);
        let (event_tx, _) = broadcast::channel::<Event>(config.event_channel_size);

        Swarm::run(
            &config.swarm_config,
            runtime_handle,
            command_rx,
            event_tx.clone(),
        )?;

        Ok(Self {
            command_tx,
            event_tx,
        })
    }

    // Create a receiver that will receive events emitted from libp2p swarm
    pub fn event_receiver(&self) -> broadcast::Receiver<Event> {
        self.event_tx.subscribe()
    }

    // Create a sender to instruct libp2p swarm to execute commands
    pub fn command_sender(&self) -> mpsc::Sender<Command> {
        self.command_tx.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn subscribe() {
        env_logger::init();

        let config1 = NomosLibp2pConfig {
            swarm_config: SwarmConfig {
                port: 60000,
                ..Default::default()
            },
            ..Default::default()
        };
        let node1 = NomosLibp2p::new(config1, tokio::runtime::Handle::current()).unwrap();

        let config2 = NomosLibp2pConfig {
            swarm_config: SwarmConfig {
                port: 60001,
                ..Default::default()
            },
            ..Default::default()
        };
        let node2 = NomosLibp2p::new(config2, tokio::runtime::Handle::current()).unwrap();

        assert!(node2
            .command_sender()
            .send(Command::Connect(
                "/ip4/127.0.0.1/tcp/60000".parse().unwrap()
            ))
            .await
            .is_ok());

        //TODO: use a fancy way instead of sleep
        tokio::time::sleep(Duration::from_secs(3)).await;

        assert!(node2
            .command_sender()
            .send(Command::Subscribe("topic1".to_string()))
            .await
            .is_ok());

        //TODO: use a fancy way instead of sleep
        tokio::time::sleep(Duration::from_secs(3)).await;

        assert!(node1
            .command_sender()
            .send(Command::Broadcast {
                topic: "topic1".to_string(),
                message: "hello".as_bytes().to_vec(),
            })
            .await
            .is_ok());

        let event = node2.event_receiver().recv().await.unwrap();
        assert!(matches!(event, Event::Message(..)));
    }
}
