use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub type TxHash = [u8; 32];
pub type BlockHash = [u8; 32];
pub type Address = [u8; 20];
pub type PeerId = String;

#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl Display for SourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AlertThresholdConfig {
    pub peer_churn_spike: u32,
    pub ingest_lag_ms: u64,
    pub decode_failure_rate_bps: u16,
    pub coverage_drop_percent: u8,
    pub storage_write_latency_ms: u32,
    pub clock_skew_ms: u32,
}

impl Default for AlertThresholdConfig {
    fn default() -> Self {
        Self {
            peer_churn_spike: 25,
            ingest_lag_ms: 500,
            decode_failure_rate_bps: 100,
            coverage_drop_percent: 40,
            storage_write_latency_ms: 250,
            clock_skew_ms: 150,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub peer_disconnects_total: u64,
    pub ingest_lag_ms: u64,
    pub tx_decode_fail_total: u64,
    pub tx_decode_total: u64,
    pub tx_per_sec_current: u64,
    pub tx_per_sec_baseline: u64,
    pub storage_write_latency_ms: u32,
    pub clock_skew_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AlertDecisions {
    pub peer_churn: bool,
    pub ingest_lag: bool,
    pub decode_failure: bool,
    pub coverage_collapse: bool,
    pub storage_latency: bool,
    pub clock_skew: bool,
}

pub fn evaluate_alerts(snapshot: &MetricSnapshot, thresholds: &AlertThresholdConfig) -> AlertDecisions {
    let decode_failure_bps = if snapshot.tx_decode_total == 0 {
        0
    } else {
        ((snapshot.tx_decode_fail_total * 10_000) / snapshot.tx_decode_total) as u16
    };

    let coverage_drop_percent = if snapshot.tx_per_sec_baseline == 0 {
        0
    } else {
        let ratio = snapshot.tx_per_sec_current as f64 / snapshot.tx_per_sec_baseline as f64;
        ((1.0 - ratio.clamp(0.0, 1.0)) * 100.0).round() as u8
    };

    AlertDecisions {
        peer_churn: snapshot.peer_disconnects_total >= thresholds.peer_churn_spike as u64,
        ingest_lag: snapshot.ingest_lag_ms >= thresholds.ingest_lag_ms,
        decode_failure: decode_failure_bps >= thresholds.decode_failure_rate_bps,
        coverage_collapse: coverage_drop_percent >= thresholds.coverage_drop_percent,
        storage_latency: snapshot.storage_write_latency_ms >= thresholds.storage_write_latency_ms,
        clock_skew: snapshot.clock_skew_ms >= thresholds.clock_skew_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate_alerts, AlertThresholdConfig, MetricSnapshot, SourceId};

    #[test]
    fn source_id_display_matches_inner_value() {
        let source = SourceId::new("peer-1");
        assert_eq!(source.to_string(), "peer-1");
    }

    #[test]
    fn evaluate_alerts_flags_expected_conditions() {
        let thresholds = AlertThresholdConfig::default();
        let snapshot = MetricSnapshot {
            peer_disconnects_total: 30,
            ingest_lag_ms: 800,
            tx_decode_fail_total: 150,
            tx_decode_total: 1_000,
            tx_per_sec_current: 200,
            tx_per_sec_baseline: 1_000,
            storage_write_latency_ms: 300,
            clock_skew_ms: 200,
        };

        let decisions = evaluate_alerts(&snapshot, &thresholds);
        assert!(decisions.peer_churn);
        assert!(decisions.ingest_lag);
        assert!(decisions.decode_failure);
        assert!(decisions.coverage_collapse);
        assert!(decisions.storage_latency);
        assert!(decisions.clock_skew);
    }
}
