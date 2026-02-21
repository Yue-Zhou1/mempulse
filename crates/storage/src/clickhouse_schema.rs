use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClickHouseSchemaConfig {
    pub retention_days: u16,
}

impl Default for ClickHouseSchemaConfig {
    fn default() -> Self {
        Self { retention_days: 14 }
    }
}

pub fn clickhouse_schema_ddl(config: ClickHouseSchemaConfig) -> Vec<String> {
    vec![
        "CREATE DATABASE IF NOT EXISTS mev_v2".to_owned(),
        clickhouse_event_table_ddl(config),
    ]
}

pub fn clickhouse_event_table_ddl(config: ClickHouseSchemaConfig) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS mev_v2.events (
    seq_id UInt64,
    ingest_ts_unix_ms Int64,
    ingest_ts_mono_ns UInt64,
    source_id String,
    payload_json String
) ENGINE = MergeTree
PARTITION BY toYYYYMM(toDateTime(ingest_ts_unix_ms / 1000))
ORDER BY (seq_id, ingest_ts_mono_ns)
TTL toDateTime(ingest_ts_unix_ms / 1000) + INTERVAL {} DAY",
        config.retention_days.max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::{ClickHouseSchemaConfig, clickhouse_event_table_ddl, clickhouse_schema_ddl};

    #[test]
    fn schema_ddl_contains_retention_interval() {
        let ddl = clickhouse_event_table_ddl(ClickHouseSchemaConfig { retention_days: 30 });
        assert!(ddl.contains("INTERVAL 30 DAY"));
    }

    #[test]
    fn schema_ddl_emits_database_and_table_statements() {
        let ddl = clickhouse_schema_ddl(ClickHouseSchemaConfig::default());
        assert_eq!(ddl.len(), 2);
        assert!(ddl[0].contains("CREATE DATABASE IF NOT EXISTS"));
        assert!(ddl[1].contains("CREATE TABLE IF NOT EXISTS"));
    }
}
