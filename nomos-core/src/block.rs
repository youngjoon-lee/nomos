// std
// crates
use bytes::{Buf, Bytes};

use crate::tx::TxCodex;
// internal

/// A block
#[derive(Clone, Debug)]
pub struct Block<Tx: TxCodex> {
    header: BlockHeader,
    transactions: Vec<Tx>,
}

/// A block header
#[derive(Copy, Clone, Debug)]
pub struct BlockHeader;

/// Identifier of a block
pub type BlockId = [u8; 32];

impl<Tx: TxCodex> Block<Tx> {
    pub fn new(header: BlockHeader, txs: impl Iterator<Item = Tx>) -> Self {
        Self {
            header,
            transactions: txs.collect(),
        }
    }

    /// Encode block into bytes
    pub fn as_bytes(&self) -> Bytes {
        Bytes::new()
    }

    pub fn header(&self) -> BlockHeader {
        self.header
    }

    pub fn transactions(&self) -> &[Tx] {
        &self.transactions
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let mut buf = bytes::BytesMut::from(b);
        let mut txns = Vec::new();
        while buf.has_remaining() {
            let tx = Tx::decode(&buf).unwrap();
            let len = tx.encoded_len();
            buf.advance(len);
            txns.push(tx);
        }

        Self {
            header: BlockHeader,
            transactions: txns,
        }
    }
}

impl BlockHeader {
    pub fn id(&self) -> BlockId {
        BlockId::default()
    }
}
