use node_runtime::NodeRuntimeBuilder;
use runtime_core::{RuntimeCoreConfig, RuntimeCoreDeps, RuntimeCoreStartArgs, RuntimeIngestMode};
use scheduler::{SchedulerConfig, scheduler_channel};
use std::sync::{Arc, Mutex, RwLock};
use storage::{InMemoryStorage, StorageWriteHandle, StorageWriteOp};
use tokio::sync::mpsc;

#[tokio::test]
async fn runtime_exposes_shutdown_handle() {
    let runtime = NodeRuntimeBuilder::default().build().expect("build");
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn runtime_builder_starts_runtime_core_and_exposes_handle_to_startup() {
    let (scheduler, _scheduler_runtime) = scheduler_channel(SchedulerConfig::default());
    let (storage_tx, _storage_rx) = mpsc::channel::<StorageWriteOp>(8);
    let startup_seen = Arc::new(Mutex::new(None));
    let startup_seen_for_closure = startup_seen.clone();

    let runtime = NodeRuntimeBuilder::default()
        .with_runtime_core_start_args(RuntimeCoreStartArgs {
            deps: RuntimeCoreDeps {
                storage: Arc::new(RwLock::new(InMemoryStorage::default())),
                writer: StorageWriteHandle::from_sender(storage_tx),
                scheduler,
            },
            config: RuntimeCoreConfig {
                ingest_mode: RuntimeIngestMode::Hybrid,
                rebuild_scheduler_from_rpc: true,
            },
        })
        .with_startup(move |runtime_core| {
            *startup_seen_for_closure.lock().expect("startup lock") = Some(runtime_core.is_some());
            Ok(None)
        })
        .build()
        .expect("build");

    assert_eq!(
        *startup_seen.lock().expect("startup lock"),
        Some(true),
        "startup should receive the runtime-core handle"
    );
    let runtime_core = runtime.runtime_core().expect("runtime core handle");
    assert_eq!(runtime_core.config().ingest_mode, RuntimeIngestMode::Hybrid);
    assert!(runtime_core.config().rebuild_scheduler_from_rpc);

    runtime.shutdown().await.expect("shutdown");
}
