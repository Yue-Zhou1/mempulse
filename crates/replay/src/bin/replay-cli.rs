use anyhow::{anyhow, Context, Result};
use event_log::EventEnvelope;
use replay::{replay_frames, ReplayMode};
use std::env;
use std::fs;

fn main() -> Result<()> {
    run_with_args(&env::args().skip(1).collect::<Vec<_>>())?;
    Ok(())
}

fn run_with_args(args: &[String]) -> Result<()> {
    let mut input_path: Option<String> = None;
    let mut output_path: Option<String> = None;
    let mut mode = ReplayMode::DeterministicEventReplay;
    let mut stride: usize = 1;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                i += 1;
                input_path = args.get(i).cloned();
            }
            "--out" => {
                i += 1;
                output_path = args.get(i).cloned();
            }
            "--mode" => {
                i += 1;
                mode = match args.get(i).map(String::as_str) {
                    Some("deterministic") => ReplayMode::DeterministicEventReplay,
                    Some("snapshot") => ReplayMode::SnapshotReplay,
                    other => return Err(anyhow!("unsupported --mode value: {other:?}")),
                };
            }
            "--stride" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--stride requires a numeric value")?;
                stride = raw.parse::<usize>().context("invalid --stride value")?;
            }
            unknown => {
                return Err(anyhow!(
                    "unknown argument '{unknown}'. expected: --input <path> [--out <path>] [--mode deterministic|snapshot] [--stride N]"
                ));
            }
        }
        i += 1;
    }

    let input_path = input_path.context("missing required argument --input <path>")?;
    let bytes = fs::read(&input_path).with_context(|| format!("read input file {input_path}"))?;
    let events: Vec<EventEnvelope> =
        serde_json::from_slice(&bytes).context("decode input event json")?;
    let frames = replay_frames(&events, mode, stride.max(1));
    let output = serde_json::to_vec_pretty(&frames).context("encode replay frames")?;

    if let Some(path) = output_path {
        fs::write(&path, output).with_context(|| format!("write output file {path}"))?;
    } else {
        println!("{}", String::from_utf8_lossy(&output));
    }

    Ok(())
}
