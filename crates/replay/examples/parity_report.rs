use common::{Address, BlockHash, SourceId, TxHash};
use event_log::{
    EventEnvelope, EventPayload, TxConfirmed, TxDecoded, TxDropped, TxReorged, TxReplaced,
};
use replay::lifecycle_parity;
use serde::Serialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
struct ParityReport {
    generated_unix_ms: i64,
    dataset_name: String,
    total_events: usize,
    total_checkpoints: usize,
    passed_checkpoints: usize,
    failed_checkpoints: usize,
    parity_percent: f64,
    slo_target_percent: f64,
    slo_pass: bool,
    failed_seq_ids: Vec<u64>,
}

fn main() -> anyhow::Result<()> {
    let out_path = resolve_out_path();
    let (events, checkpoints) = synthetic_event_window();
    let parity = lifecycle_parity(&events, &checkpoints);
    let passed_checkpoints = parity.values().filter(|passed| **passed).count();
    let failed_seq_ids = parity
        .iter()
        .filter_map(|(seq_id, passed)| if *passed { None } else { Some(*seq_id) })
        .collect::<Vec<_>>();
    let total_checkpoints = checkpoints.len();
    let parity_percent = if total_checkpoints == 0 {
        0.0
    } else {
        (passed_checkpoints as f64 / total_checkpoints as f64) * 100.0
    };
    let report = ParityReport {
        generated_unix_ms: unix_ms_now(),
        dataset_name: "synthetic-mixed-lifecycle-v1".to_owned(),
        total_events: events.len(),
        total_checkpoints,
        passed_checkpoints,
        failed_checkpoints: total_checkpoints.saturating_sub(passed_checkpoints),
        parity_percent,
        slo_target_percent: 99.99,
        slo_pass: parity_percent >= 99.99,
        failed_seq_ids,
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out_path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "parity-report: events={} checkpoints={} pass_rate={:.4}% out={}",
        report.total_events,
        report.total_checkpoints,
        report.parity_percent,
        out_path.display()
    );
    Ok(())
}

fn resolve_out_path() -> PathBuf {
    let args = env::args().collect::<Vec<_>>();
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "--out" && idx + 1 < args.len() {
            return PathBuf::from(&args[idx + 1]);
        }
        idx += 1;
    }
    PathBuf::from("artifacts/replay/parity_report.json")
}

fn synthetic_event_window() -> (Vec<EventEnvelope>, Vec<u64>) {
    let source_id = SourceId::new("parity-example");
    let mut events = Vec::new();
    let mut checkpoints = Vec::new();
    let mut seq_id = 1_u64;
    let mut mono_ns = 10_000_u64;
    let base_ts = 1_710_000_000_000_i64;

    for i in 0..1_000_u64 {
        let hash = hash_from_u64(i + 1);
        let sender = address_from_u64(i + 101);

        push_event(
            &mut events,
            &mut checkpoints,
            envelope(
                seq_id,
                base_ts + seq_id as i64,
                mono_ns,
                source_id.clone(),
                EventPayload::TxDecoded(TxDecoded {
                    hash,
                    tx_type: 2,
                    sender,
                    nonce: i,
                    chain_id: Some(1),
                    to: Some(address_from_u64(i + 201)),
                    value_wei: Some(1_000_000_000_000_000),
                    gas_limit: Some(220_000),
                    gas_price_wei: Some(35_000_000_000),
                    max_fee_per_gas_wei: Some(45_000_000_000),
                    max_priority_fee_per_gas_wei: Some(3_000_000_000),
                    max_fee_per_blob_gas_wei: None,
                    calldata_len: Some(164),
                }),
            ),
        );
        seq_id = seq_id.saturating_add(1);
        mono_ns = mono_ns.saturating_add(100);

        if i % 3 == 0 {
            push_event(
                &mut events,
                &mut checkpoints,
                envelope(
                    seq_id,
                    base_ts + seq_id as i64,
                    mono_ns,
                    source_id.clone(),
                    EventPayload::TxConfirmedFinal(TxConfirmed {
                        hash,
                        block_number: 20_000_000 + i,
                        block_hash: block_from_u64(i + 11),
                    }),
                ),
            );
            seq_id = seq_id.saturating_add(1);
            mono_ns = mono_ns.saturating_add(100);
        }

        if i % 5 == 0 {
            push_event(
                &mut events,
                &mut checkpoints,
                envelope(
                    seq_id,
                    base_ts + seq_id as i64,
                    mono_ns,
                    source_id.clone(),
                    EventPayload::TxDropped(TxDropped {
                        hash,
                        reason: "evicted_low_tip".to_owned(),
                    }),
                ),
            );
            seq_id = seq_id.saturating_add(1);
            mono_ns = mono_ns.saturating_add(100);
        }

        if i % 7 == 0 {
            let replaced_by = hash_from_u64(10_000 + i);
            push_event(
                &mut events,
                &mut checkpoints,
                envelope(
                    seq_id,
                    base_ts + seq_id as i64,
                    mono_ns,
                    source_id.clone(),
                    EventPayload::TxReplaced(TxReplaced { hash, replaced_by }),
                ),
            );
            seq_id = seq_id.saturating_add(1);
            mono_ns = mono_ns.saturating_add(100);

            push_event(
                &mut events,
                &mut checkpoints,
                envelope(
                    seq_id,
                    base_ts + seq_id as i64,
                    mono_ns,
                    source_id.clone(),
                    EventPayload::TxDecoded(TxDecoded {
                        hash: replaced_by,
                        tx_type: 2,
                        sender,
                        nonce: i,
                        chain_id: Some(1),
                        to: Some(address_from_u64(i + 301)),
                        value_wei: Some(1_100_000_000_000_000),
                        gas_limit: Some(250_000),
                        gas_price_wei: Some(40_000_000_000),
                        max_fee_per_gas_wei: Some(55_000_000_000),
                        max_priority_fee_per_gas_wei: Some(5_000_000_000),
                        max_fee_per_blob_gas_wei: None,
                        calldata_len: Some(188),
                    }),
                ),
            );
            seq_id = seq_id.saturating_add(1);
            mono_ns = mono_ns.saturating_add(100);
        }

        if i % 11 == 0 {
            push_event(
                &mut events,
                &mut checkpoints,
                envelope(
                    seq_id,
                    base_ts + seq_id as i64,
                    mono_ns,
                    source_id.clone(),
                    EventPayload::TxReorged(TxReorged {
                        hash,
                        old_block_hash: block_from_u64(i + 31),
                        new_block_hash: block_from_u64(i + 32),
                    }),
                ),
            );
            seq_id = seq_id.saturating_add(1);
            mono_ns = mono_ns.saturating_add(100);
        }
    }

    (events, checkpoints)
}

fn envelope(
    seq_id: u64,
    ts_unix_ms: i64,
    ts_mono_ns: u64,
    source_id: SourceId,
    payload: EventPayload,
) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: ts_unix_ms,
        ingest_ts_mono_ns: ts_mono_ns,
        source_id,
        payload,
    }
}

fn push_event(events: &mut Vec<EventEnvelope>, checkpoints: &mut Vec<u64>, event: EventEnvelope) {
    checkpoints.push(event.seq_id);
    events.push(event);
}

fn hash_from_u64(value: u64) -> TxHash {
    let mut out = [0_u8; 32];
    for (idx, byte) in out.iter_mut().enumerate() {
        let shift = ((idx % 8) * 8) as u32;
        let salt = (idx as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        *byte = value.wrapping_add(salt).rotate_left((idx % 13) as u32).wrapping_shr(shift) as u8;
    }
    out
}

fn address_from_u64(value: u64) -> Address {
    let mut out = [0_u8; 20];
    let bytes = hash_from_u64(value);
    out.copy_from_slice(&bytes[..20]);
    out
}

fn block_from_u64(value: u64) -> BlockHash {
    hash_from_u64(value.wrapping_add(50_000))
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
