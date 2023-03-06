use nomos_consensus::{
    network::{adapters::mock::MockAdapter as ConsensusMockAdapter, messages::ApprovalMsg},
    overlay::flat::Flat,
    CarnotConsensus, CarnotSettings,
};
use nomos_core::{
    block::{BlockHeader, BlockId},
    fountain::mock::MockFountain,
    tx::mock::{MockTransaction, MockTransactionMsg, MockTxId},
};
use nomos_log::{Logger, LoggerSettings};
use nomos_network::{
    backends::mock::{Mock, MockConfig, MockMessage},
    NetworkConfig, NetworkService,
};

use overwatch_derive::*;
use overwatch_rs::{overwatch::OverwatchRunner, services::handle::ServiceHandle};

use nomos_mempool::{
    backend::mockpool::MockPool,
    network::adapters::mock::{MockAdapter, MOCK_TX_CONTENT_TOPIC},
    MempoolMsg, MempoolService,
};

#[derive(Services)]
struct MockPoolNode {
    logging: ServiceHandle<Logger>,
    network: ServiceHandle<NetworkService<Mock>>,
    mockpool: ServiceHandle<MempoolService<MockAdapter, MockPool<MockTxId, MockTransactionMsg>>>,
    #[allow(clippy::type_complexity)]
    consensus: ServiceHandle<
        CarnotConsensus<
            ConsensusMockAdapter,
            MockPool<MockTxId, MockTransactionMsg>,
            MockAdapter,
            MockFountain,
            Flat<MockTransactionMsg>,
        >,
    >,
}

#[test]
fn test_carnot() {
    const KEY: [u8; 32] = [0; 32];

    let predefined_messages = vec![MockMessage {
        payload: "This is foo".to_string(),
        content_topic: MOCK_TX_CONTENT_TOPIC,
        version: 0,
        timestamp: 0,
    }];

    let app = OverwatchRunner::<MockPoolNode>::run(
        MockPoolNodeServiceSettings {
            network: NetworkConfig {
                backend: MockConfig {
                    predefined_messages,
                    duration: tokio::time::Duration::from_millis(100),
                    seed: 0,
                    version: 1,
                    weights: None,
                },
            },
            mockpool: (),
            logging: LoggerSettings::default(),
            consensus: CarnotSettings::new(Default::default(), ()),
        },
        None,
    )
    .map_err(|e| eprintln!("Error encountered: {}", e))
    .unwrap();

    let mempool = app
        .handle()
        .relay::<MempoolService<MockAdapter, MockPool<MockTxId, MockTransactionMsg>>>();

    app.spawn(async move {
        let mempool_outbound = mempool.connect().await.unwrap();

        // wait the first trasaction to be processed
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let (mtx, mrx) = tokio::sync::oneshot::channel();
        mempool_outbound
            .send(MempoolMsg::View {
                ancestor_hint: BlockId::default(),
                reply_channel: mtx,
            })
            .await
            .unwrap();

        // send view message to mempool, and check if the previous transaction has been in the pending txs
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let items = mrx
            .await
            .unwrap()
            .into_iter()
            .filter_map(|tx| {
                if let MockTransactionMsg::Response(msg) = tx {
                    Some(msg)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // if !items.is_empty() {
        //     assert_eq!(items.len(), 1);
        //     assert_eq!(
        //         items[0],
        //         MockMessage {
        //             payload: String::from_utf8_lossy(&ApprovalMsg::from_bytes(&KEY).as_bytes())
        //                 .to_string(),
        //             content_topic: MOCK_APPROVAL_CONTENT_TOPIC,
        //             version: 1,
        //             timestamp: 0,
        //         }
        //     );
        // }

        // now the first transcation is in the mempool, we can mark it as in block
        let txid = MockTxId::from(&MockTransactionMsg::Request(MockTransaction::from(
            &MockMessage {
                payload: "This is foo".to_string(),
                content_topic: MOCK_TX_CONTENT_TOPIC,
                version: 0,
                timestamp: 0,
            },
        )));
        mempool_outbound
            .send(MempoolMsg::MarkInBlock {
                ids: vec![txid],
                block: nomos_core::block::BlockHeader,
            })
            .await
            .unwrap();

        // wait the mempool move from the transaction to the in_block_txs
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // send block view message to mempool, and check if the previous transaction has been in the in_block_txs
        let (mtx, mrx) = tokio::sync::oneshot::channel();
        mempool_outbound
            .send(MempoolMsg::BlockTransactions {
                block: BlockHeader.id(),
                reply_channel: mtx,
            })
            .await
            .unwrap();

        let items = mrx
            .await
            .unwrap()
            .into_iter()
            .filter_map(|tx| {
                if let MockTransactionMsg::Request(msg) = tx {
                    Some(msg)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // assert_eq!(
        //     items,
        //     vec![MockMessage {
        //         payload: "This is foo".to_string(),
        //         content_topic: MOCK_TX_CONTENT_TOPIC,
        //         version: 0,
        //         timestamp: 0,
        //     }]
        // )
    });

    // wait the app thread finish
    std::thread::sleep(std::time::Duration::from_secs(5));
}
