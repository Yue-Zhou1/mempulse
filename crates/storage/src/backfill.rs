use crate::{EventStore, InMemoryStorage};
use event_log::{EventEnvelope, sort_deterministic};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackfillConfig {
    pub retention_window_ms: i64,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            retention_window_ms: 7 * 24 * 60 * 60 * 1000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackfillSummary {
    pub inserted: u64,
    pub skipped_duplicates: u64,
    pub pruned_by_retention: u64,
}

pub struct BackfillWriter {
    storage: Arc<RwLock<InMemoryStorage>>,
    config: BackfillConfig,
    seen_seq_ids: std::collections::BTreeSet<u64>,
}

impl BackfillWriter {
    pub fn new(storage: Arc<RwLock<InMemoryStorage>>, config: BackfillConfig) -> Self {
        Self {
            storage,
            config: BackfillConfig {
                retention_window_ms: config.retention_window_ms.max(1),
            },
            seen_seq_ids: std::collections::BTreeSet::new(),
        }
    }

    pub fn apply_events(&mut self, events: &[EventEnvelope], now_unix_ms: i64) -> BackfillSummary {
        let retention_cutoff = now_unix_ms.saturating_sub(self.config.retention_window_ms);
        let mut sorted = events.to_vec();
        sort_deterministic(&mut sorted);

        let mut summary = BackfillSummary::default();
        for event in sorted {
            if event.ingest_ts_unix_ms < retention_cutoff {
                summary.pruned_by_retention = summary.pruned_by_retention.saturating_add(1);
                continue;
            }
            if !self.seen_seq_ids.insert(event.seq_id) {
                summary.skipped_duplicates = summary.skipped_duplicates.saturating_add(1);
                continue;
            }

            if let Ok(mut storage) = self.storage.write() {
                storage.append_event(event);
                summary.inserted = summary.inserted.saturating_add(1);
            }
        }
        summary
    }
}
