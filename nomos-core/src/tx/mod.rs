#[cfg(feature = "mock")]
pub mod mock;
mod transaction;

use serde::{Deserialize, Serialize};
pub use transaction::Transaction;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Tx {
    Transfer(Transaction),
}

pub trait TxCodex {
    type Error: std::error::Error + Send + Sync + 'static;

    fn encode(&self) -> bytes::Bytes;

    fn encoded_len(&self) -> usize;

    fn decode(src: &[u8]) -> Result<Self, Self::Error>
    where
        Self: Sized;
}
