// std
use std::marker::PhantomData;
// crates
// internal
use nomos_core::{block::BlockHeader, crypto::PrivateKey, tx::TxCodex};
use nomos_mempool::MempoolMsg;

use super::*;

// TODO: take care of sensitve material
struct Enclave {
    key: PrivateKey,
}

pub struct Leadership<Tx, Id> {
    key: Enclave,
    mempool: OutboundRelay<MempoolMsg<Tx, Id>>,
}

pub enum LeadershipResult<'view, Tx: TxCodex> {
    Leader {
        block: Block<Tx>,
        _view: PhantomData<&'view u8>,
    },
    NotLeader {
        _view: PhantomData<&'view u8>,
    },
}

impl<Tx: TxCodex, Id> Leadership<Tx, Id> {
    pub fn new(key: PrivateKey, mempool: OutboundRelay<MempoolMsg<Tx, Id>>) -> Self {
        Self {
            key: Enclave { key },
            mempool,
        }
    }

    #[allow(unused, clippy::diverging_sub_expression)]
    pub async fn try_propose_block<'view>(
        &self,
        view: &'view View,
        tip: &Tip,
        qc: Approval,
    ) -> LeadershipResult<'view, Tx> {
        // let ancestor_hint = todo!("get the ancestor from the tip");
        // TODO: use the correct ancestor hint
        let fake_ancestor_hint = [0u8; 32];
        if view.is_leader(self.key.key) {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if let Err((e, _msg)) = self
                .mempool
                .send(MempoolMsg::View {
                    ancestor_hint: fake_ancestor_hint,
                    reply_channel: tx,
                })
                .await
            {
                tracing::error!(err=%e);
                panic!("{e}");
            }

            let iter = match rx.await {
                Ok(iter) => iter,
                Err(e) => {
                    tracing::error!("{}", e);
                    panic!("{e}");
                }
            };

            LeadershipResult::Leader {
                _view: PhantomData,
                block: Block::<Tx>::new(BlockHeader, iter),
            }
        } else {
            LeadershipResult::NotLeader { _view: PhantomData }
        }
    }
}
