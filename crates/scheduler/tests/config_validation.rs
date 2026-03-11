use scheduler::{SchedulerConfig, SchedulerConfigError, scheduler_channel};

#[test]
fn scheduler_channel_rejects_zero_handoff_queue_capacity() {
    let error = scheduler_channel(SchedulerConfig {
        handoff_queue_capacity: 0,
        max_pending_per_sender: 64,
        replacement_fee_bump_bps: 1_000,
    })
    .expect_err("zero handoff queue capacity should be rejected");

    assert_eq!(error, SchedulerConfigError::HandoffQueueCapacityZero);
}

#[test]
fn scheduler_channel_rejects_zero_max_pending_per_sender() {
    let error = scheduler_channel(SchedulerConfig {
        handoff_queue_capacity: 16,
        max_pending_per_sender: 0,
        replacement_fee_bump_bps: 1_000,
    })
    .expect_err("zero max pending per sender should be rejected");

    assert_eq!(error, SchedulerConfigError::MaxPendingPerSenderZero);
}
