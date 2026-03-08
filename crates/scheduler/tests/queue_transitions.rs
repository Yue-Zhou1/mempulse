use common::{Address, SourceId};
use event_log::TxDecoded;
use scheduler::{
    SchedulerAdmission, SchedulerConfig, SchedulerQueueState, SchedulerQueueTransition,
    ValidatedTransaction, scheduler_channel,
};

fn sample_validated_tx(
    hash_seed: u8,
    sender: Address,
    nonce: u64,
    max_fee_per_gas_wei: u128,
    source: &str,
    observed_at_unix_ms: i64,
    observed_at_mono_ns: u64,
) -> ValidatedTransaction {
    ValidatedTransaction {
        source_id: SourceId::new(source),
        observed_at_unix_ms,
        observed_at_mono_ns,
        decoded: TxDecoded {
            hash: [hash_seed; 32],
            tx_type: 2,
            sender,
            nonce,
            chain_id: Some(1),
            to: Some([hash_seed.saturating_add(1); 20]),
            value_wei: Some(42),
            gas_limit: Some(21_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(max_fee_per_gas_wei),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
    }
}

fn sender(seed: u8) -> Address {
    [seed; 20]
}

#[tokio::test]
async fn scheduler_admit_outcome_reports_sender_queue_transitions() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender = sender(0xa1);
    let nonce_7 = sample_validated_tx(10, sender, 7, 100, "rpc-mainnet", 1_700_000_000_010, 10);
    let nonce_9 = sample_validated_tx(11, sender, 9, 100, "rpc-mainnet", 1_700_000_000_011, 11);
    let nonce_8 = sample_validated_tx(12, sender, 8, 100, "rpc-mainnet", 1_700_000_000_012, 12);

    let first = handle
        .admit_outcome(nonce_7.clone())
        .await
        .expect("admit nonce 7");
    assert_eq!(first.admission, SchedulerAdmission::Admitted);
    assert_eq!(
        first.queue_transitions,
        vec![SchedulerQueueTransition {
            hash: nonce_7.hash(),
            sender,
            nonce: 7,
            state: SchedulerQueueState::Ready,
        }]
    );

    let second = handle
        .admit_outcome(nonce_9.clone())
        .await
        .expect("admit nonce 9");
    assert_eq!(second.admission, SchedulerAdmission::Admitted);
    assert_eq!(
        second.queue_transitions,
        vec![SchedulerQueueTransition {
            hash: nonce_9.hash(),
            sender,
            nonce: 9,
            state: SchedulerQueueState::Blocked { expected_nonce: 8 },
        }]
    );

    let third = handle
        .admit_outcome(nonce_8.clone())
        .await
        .expect("admit nonce 8");
    assert_eq!(third.admission, SchedulerAdmission::Admitted);
    assert_eq!(
        third.queue_transitions,
        vec![
            SchedulerQueueTransition {
                hash: nonce_8.hash(),
                sender,
                nonce: 8,
                state: SchedulerQueueState::Ready,
            },
            SchedulerQueueTransition {
                hash: nonce_9.hash(),
                sender,
                nonce: 9,
                state: SchedulerQueueState::Ready,
            },
        ]
    );

    runtime_task.abort();
}
