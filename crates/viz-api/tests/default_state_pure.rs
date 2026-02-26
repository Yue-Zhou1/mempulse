use viz_api::default_state;
use viz_api::live_rpc::{live_rpc_feed_start_count, reset_live_rpc_feed_start_count};

#[tokio::test]
async fn default_state_is_pure_and_does_not_spawn_ingest() {
    reset_live_rpc_feed_start_count();
    let _state = default_state();
    assert_eq!(live_rpc_feed_start_count(), 0);
}
