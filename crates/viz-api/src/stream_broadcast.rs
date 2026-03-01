use crate::StreamV2Dispatch;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DashboardStreamBroadcastEvent {
    Delta(Box<StreamV2Dispatch>),
    Reset { reason: String, latest_seq_id: u64 },
}

impl DashboardStreamBroadcastEvent {
    pub fn seq_id(&self) -> u64 {
        match self {
            DashboardStreamBroadcastEvent::Delta(dispatch) => {
                dispatch.seq.max(dispatch.watermark.latest_ingest_seq)
            }
            DashboardStreamBroadcastEvent::Reset { latest_seq_id, .. } => *latest_seq_id,
        }
    }
}

#[derive(Debug)]
pub struct DashboardStreamBroadcaster {
    sender: broadcast::Sender<DashboardStreamBroadcastEvent>,
    replay: Mutex<VecDeque<DashboardStreamBroadcastEvent>>,
    replay_capacity: usize,
    latest_seq_id: AtomicU64,
}

impl DashboardStreamBroadcaster {
    pub fn new(replay_capacity: usize, channel_capacity: usize) -> Self {
        let normalized_replay_capacity = replay_capacity.max(1);
        let normalized_channel_capacity = channel_capacity.max(1);
        let (sender, _) = broadcast::channel(normalized_channel_capacity);
        Self {
            sender,
            replay: Mutex::new(VecDeque::with_capacity(normalized_replay_capacity)),
            replay_capacity: normalized_replay_capacity,
            latest_seq_id: AtomicU64::new(0),
        }
    }

    pub fn latest_seq_id(&self) -> u64 {
        self.latest_seq_id.load(Ordering::Relaxed)
    }

    pub fn publish_delta(&self, dispatch: StreamV2Dispatch) {
        let event = DashboardStreamBroadcastEvent::Delta(Box::new(dispatch));
        self.publish_event(event);
    }

    pub fn publish_reset(&self, reason: &str, latest_seq_id: u64) {
        let event = DashboardStreamBroadcastEvent::Reset {
            reason: reason.to_owned(),
            latest_seq_id,
        };
        self.publish_event(event);
    }

    pub fn subscribe_from(
        &self,
        after_seq_id: u64,
    ) -> (
        Vec<DashboardStreamBroadcastEvent>,
        broadcast::Receiver<DashboardStreamBroadcastEvent>,
    ) {
        let replay_snapshot = self
            .replay
            .lock()
            .map(|guard| guard.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let latest_seq_id = self.latest_seq_id();

        let mut initial = Vec::new();
        if let Some(first_event) = replay_snapshot.first() {
            let first_seq_id = first_event.seq_id();
            if after_seq_id > 0 && first_seq_id > after_seq_id.saturating_add(1) {
                initial.push(DashboardStreamBroadcastEvent::Reset {
                    reason: "gap".to_owned(),
                    latest_seq_id: latest_seq_id.max(first_seq_id),
                });
            } else {
                initial.extend(
                    replay_snapshot
                        .into_iter()
                        .filter(|event| event.seq_id() > after_seq_id),
                );
            }
        }

        (initial, self.sender.subscribe())
    }

    fn publish_event(&self, event: DashboardStreamBroadcastEvent) {
        let seq_id = event.seq_id();
        self.latest_seq_id.fetch_max(seq_id, Ordering::Relaxed);
        self.push_replay(event.clone());
        let _ = self.sender.send(event);
    }

    fn push_replay(&self, event: DashboardStreamBroadcastEvent) {
        if let Ok(mut replay) = self.replay.lock() {
            if replay.len() >= self.replay_capacity {
                let _ = replay.pop_front();
            }
            replay.push_back(event);
        }
    }
}
