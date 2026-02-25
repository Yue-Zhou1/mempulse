use viz_api::live_rpc::{
    coalesce_hash_batches, dispatchable_batch_count, retry_backoff_delay_ms, rotate_endpoint_index,
};

fn sample_hashes(n: usize) -> Vec<String> {
    (0..n).map(|index| format!("0x{:064x}", index + 1)).collect()
}

#[test]
fn live_rpc_batch_fetch_enforces_batch_size_limit() {
    let hashes = sample_hashes(10);
    let batches = coalesce_hash_batches(&hashes, 3);

    assert_eq!(batches.len(), 4);
    assert!(batches.iter().all(|batch| batch.len() <= 3));
    assert_eq!(
        batches.iter().map(|batch| batch.len()).sum::<usize>(),
        hashes.len()
    );
}

#[test]
fn live_rpc_batch_fetch_bounds_in_flight_dispatch_per_chain() {
    assert_eq!(dispatchable_batch_count(100, 10, 3), 3);
    assert_eq!(dispatchable_batch_count(5, 10, 3), 1);
    assert_eq!(dispatchable_batch_count(0, 10, 3), 0);
}

#[test]
fn live_rpc_batch_fetch_retry_and_fallback_endpoint_behavior() {
    assert_eq!(retry_backoff_delay_ms(100, 0), 100);
    assert_eq!(retry_backoff_delay_ms(100, 1), 200);
    assert_eq!(retry_backoff_delay_ms(100, 2), 400);

    assert_eq!(rotate_endpoint_index(0, 2), 1);
    assert_eq!(rotate_endpoint_index(1, 2), 0);
}
