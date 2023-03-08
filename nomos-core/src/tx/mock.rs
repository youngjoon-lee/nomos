use super::TxCodex;
use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use bytes::{Buf, BufMut};

const TX_ID_LEN: usize = core::mem::size_of::<MockTxId>();

#[derive(Debug, Clone, Eq, PartialEq, Hash, serde::Serialize)]
pub struct MockTransaction {
    id: MockTxId,
    content: bytes::Bytes,
}

impl TxCodex for MockTransactionMsg {
    type Error = std::convert::Infallible;

    fn encode(&self) -> bytes::Bytes {
        let mut buf = bytes::BytesMut::new();
        // Encoding: | type 1 bit | 8 bits content len | 32 bits id | content |
        match self {
            MockTransactionMsg::Request(msg) => {
                buf.put_u8(0);
                buf.put_slice(msg.to_bytes().as_ref());
            }
            MockTransactionMsg::Response(msg) => {
                buf.put_u8(1);
                buf.put_slice(msg.to_bytes().as_ref());
            }
        }
        buf.freeze()
    }

    fn encoded_len(&self) -> usize {
        1 + 8
            + TX_ID_LEN
            + match self {
                MockTransactionMsg::Request(msg) | MockTransactionMsg::Response(msg) => {
                    msg.content.len()
                }
            }
    }

    fn decode(src: &[u8]) -> Result<Self, Self::Error> {
        Ok(match src[0] {
            0 => Self::Request(MockTransaction::from_slice(&src[1..])),
            1 => Self::Response(MockTransaction::from_slice(&src[1..])),
            _ => panic!("Invalid transaction message type"),
        })
    }
}

impl MockTransaction {
    pub fn to_bytes(&self) -> bytes::Bytes {
        let mut buf = bytes::BytesMut::new();
        // Encoding: | 8 bits content len| 32 bits id | content |
        buf.put_u64(self.content.len() as u64);
        buf.put_slice(self.id.as_ref());
        buf.put_slice(self.content.as_ref());
        buf.freeze()
    }

    pub fn from_slice(s: &[u8]) -> Self {
        let mut buf = bytes::BytesMut::from(s);
        let len = buf.get_u64();
        let mut id = [0u8; 32];
        id.copy_from_slice(&s[8..40]);
        let id = MockTxId(id);
        Self {
            id,
            content: bytes::Bytes::copy_from_slice(&s[40..40 + len as usize]),
        }
    }
}

impl From<&nomos_network::backends::mock::MockMessage> for MockTransaction {
    fn from(msg: &nomos_network::backends::mock::MockMessage) -> Self {
        let id = MockTxId::from(msg);
        Self {
            id,
            content: msg.to_bytes(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub enum MockTransactionMsg {
    Request(MockTransaction),
    Response(MockTransaction),
}

#[derive(Debug, Eq, Hash, PartialEq, Ord, Copy, Clone, PartialOrd, serde::Serialize)]
pub struct MockTxId([u8; 32]);

impl From<[u8; 32]> for MockTxId {
    fn from(tx_id: [u8; 32]) -> Self {
        Self(tx_id)
    }
}

impl core::ops::Deref for MockTxId {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for MockTxId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl MockTxId {
    pub fn new(tx_id: [u8; 32]) -> MockTxId {
        MockTxId(tx_id)
    }
}

impl From<&nomos_network::backends::mock::MockMessage> for MockTxId {
    fn from(msg: &nomos_network::backends::mock::MockMessage) -> Self {
        let mut hasher = Blake2bVar::new(32).unwrap();
        hasher.update(msg.to_bytes().as_ref());
        let mut id = [0u8; 32];
        hasher.finalize_variable(&mut id).unwrap();
        Self(id)
    }
}

impl From<&MockTransactionMsg> for MockTxId {
    fn from(msg: &MockTransactionMsg) -> Self {
        match msg {
            MockTransactionMsg::Request(tx) | MockTransactionMsg::Response(tx) => tx.id,
        }
    }
}
