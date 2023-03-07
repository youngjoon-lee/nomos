use nomos_consensus::{
    network::adapters::mock::{MockAdapter as ConsensusMockAdapter, MOCK_BLOCK_CONTENT_TOPIC},
    overlay::flat::Flat,
    CarnotConsensus, CarnotSettings,
};
use nomos_core::{
    block::BlockHeader,
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
    let predefined_messages = vec![
        MockMessage {
            payload: String::from_utf8_lossy(&[0; 32]).to_string(),
            content_topic: MOCK_BLOCK_CONTENT_TOPIC,
            version: 0,
            timestamp: 0,
        },
        MockMessage {
            payload: String::from_utf8_lossy(&[0; 32]).to_string(),
            content_topic: MOCK_TX_CONTENT_TOPIC,
            version: 0,
            timestamp: 0,
        },
        MockMessage {
            payload: String::from_utf8_lossy(&[1; 32]).to_string(),
            content_topic: MOCK_TX_CONTENT_TOPIC,
            version: 0,
            timestamp: 0,
        },
    ];

    let expected = vec![
        MockTransaction::from(&MockMessage {
            payload: String::from_utf8_lossy(&[0; 32]).to_string(),
            content_topic: MOCK_TX_CONTENT_TOPIC,
            version: 0,
            timestamp: 0,
        }),
        MockTransaction::from(&MockMessage {
            payload: String::from_utf8_lossy(&[1; 32]).to_string(),
            content_topic: MOCK_TX_CONTENT_TOPIC,
            version: 0,
            timestamp: 0,
        }),
    ];

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
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // send block transactions message to mempool, and check if the previous transaction has been in the in_block_txs
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
            .filter_map(|tx| {
                if let MockTransactionMsg::Request(msg) = tx {
                    Some(msg)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(2, items.len());
        assert_eq!(items, expected)
    });

    // wait the app thread finish
    std::thread::sleep(std::time::Duration::from_secs(5));
}
