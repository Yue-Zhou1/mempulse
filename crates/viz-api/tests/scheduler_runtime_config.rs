use common::{Address, SourceId};
use event_log::TxDecoded;
use parking_lot::RwLock;
use scheduler::{SchedulerAdmission, ValidatedTransaction};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use storage::InMemoryStorage;
use tokio::time::{Duration, Instant, sleep};
use viz_api::runtime_bootstrap_from_storage;

const ENV_SCHEDULER_HANDOFF_QUEUE_CAPACITY: &str = "VIZ_API_SCHEDULER_HANDOFF_QUEUE_CAPACITY";
const ENV_SCHEDULER_MAX_PENDING_PER_SENDER: &str = "VIZ_API_SCHEDULER_MAX_PENDING_PER_SENDER";
const ENV_SCHEDULER_REPLACEMENT_FEE_BUMP_BPS: &str = "VIZ_API_SCHEDULER_REPLACEMENT_FEE_BUMP_BPS";

fn scheduler_env_mutex() -> &'static Mutex<()> {
    static MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    MUTEX.get_or_init(|| Mutex::new(()))
}

struct SchedulerEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
}

impl SchedulerEnvGuard {
    fn set(overrides: &[(&'static str, &str)]) -> Self {
        let lock = scheduler_env_mutex()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut saved = Vec::with_capacity(overrides.len());
        for (key, value) in overrides {
            saved.push((*key, std::env::var(key).ok()));
            unsafe {
                std::env::set_var(key, value);
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for SchedulerEnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.iter().rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
    }
}

fn sender(seed: u8) -> Address {
    [seed; 20]
}

fn sample_validated_tx(
    hash_seed: u8,
    sender: Address,
    nonce: u64,
    max_fee: u128,
) -> ValidatedTransaction {
    ValidatedTransaction {
        source_id: SourceId::new("rpc-mainnet"),
        observed_at_unix_ms: 1_700_000_000_000 + hash_seed as i64,
        observed_at_mono_ns: hash_seed as u64,
        calldata: vec![hash_seed; 4],
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
            max_fee_per_gas_wei: Some(max_fee),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
    }
}

async fn wait_for(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met before timeout");
}

#[tokio::test]
async fn runtime_bootstrap_reads_scheduler_queue_bounds_from_env() {
    let _env = SchedulerEnvGuard::set(&[
        (ENV_SCHEDULER_HANDOFF_QUEUE_CAPACITY, "3"),
        (ENV_SCHEDULER_MAX_PENDING_PER_SENDER, "2"),
    ]);

    let bootstrap =
        runtime_bootstrap_from_storage(Arc::new(RwLock::new(InMemoryStorage::default())));
    let scheduler = bootstrap.scheduler.clone();
    let sender = sender(0xa1);

    assert_eq!(scheduler.metrics().handoff_queue_capacity, 3);

    scheduler
        .admit(sample_validated_tx(1, sender, 1, 100))
        .await
        .expect("admit nonce 1");
    scheduler
        .admit(sample_validated_tx(2, sender, 2, 100))
        .await
        .expect("admit nonce 2");
    scheduler
        .admit(sample_validated_tx(3, sender, 3, 100))
        .await
        .expect("admit nonce 3");

    wait_for(|| {
        let metrics = scheduler.metrics();
        metrics.pending_total == 2 && metrics.sender_limit_drop_total == 1
    })
    .await;
}

#[tokio::test]
async fn runtime_bootstrap_reads_scheduler_replacement_bump_from_env() {
    let _env = SchedulerEnvGuard::set(&[(ENV_SCHEDULER_REPLACEMENT_FEE_BUMP_BPS, "2000")]);

    let bootstrap =
        runtime_bootstrap_from_storage(Arc::new(RwLock::new(InMemoryStorage::default())));
    let scheduler = bootstrap.scheduler.clone();
    let sender = sender(0xb2);

    let incumbent = sample_validated_tx(10, sender, 7, 100);
    let underpriced = sample_validated_tx(11, sender, 7, 119);
    let replacement = sample_validated_tx(12, sender, 7, 120);

    assert_eq!(
        scheduler.admit(incumbent).await.expect("admit incumbent"),
        SchedulerAdmission::Admitted
    );
    assert_eq!(
        scheduler
            .admit(underpriced)
            .await
            .expect("admit underpriced replacement"),
        SchedulerAdmission::UnderpricedReplacement
    );
    assert_eq!(
        scheduler
            .admit(replacement)
            .await
            .expect("admit valid replacement"),
        SchedulerAdmission::Replaced {
            replaced_hash: [10; 32],
        }
    );
}
