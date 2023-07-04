use multiaddr::Multiaddr;

#[derive(Debug)]
pub enum Command {
    Connect(Multiaddr),
    Broadcast { topic: Topic, message: Vec<u8> },
    Subscribe(Topic),
    Unsubscribe(Topic),
}

pub type Topic = String;
