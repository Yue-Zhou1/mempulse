use anyhow::{Context, Result};
use event_log::EventEnvelope;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StorageWal {
    path: PathBuf,
}

impl StorageWal {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create WAL parent directory {}", parent.display()))?;
        }
        if !path.exists() {
            fs::write(&path, b"")
                .with_context(|| format!("initialize WAL file {}", path.display()))?;
        }
        Ok(Self { path })
    }

    pub fn append_event(&self, event: &EventEnvelope) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open WAL file {} for append", self.path.display()))?;
        let encoded = serde_json::to_string(event).context("serialize WAL event")?;
        writeln!(file, "{encoded}")
            .with_context(|| format!("append WAL event to {}", self.path.display()))?;
        Ok(())
    }

    pub fn recover_events(&self) -> Result<Vec<EventEnvelope>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .with_context(|| format!("open WAL file {} for recovery", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.context("read WAL line")?;
            if line.trim().is_empty() {
                continue;
            }
            let event: EventEnvelope =
                serde_json::from_str(&line).context("decode WAL event line")?;
            events.push(event);
        }
        Ok(events)
    }

    pub fn scan(&self, from_seq_id: u64, limit: usize) -> Result<Vec<EventEnvelope>> {
        let limit = limit.max(1);
        let mut events = self.recover_events()?;
        events.sort_by_key(|event| event.seq_id);
        Ok(events
            .into_iter()
            .filter(|event| event.seq_id > from_seq_id)
            .take(limit)
            .collect())
    }

    pub fn clear(&self) -> Result<()> {
        fs::write(&self.path, b"")
            .with_context(|| format!("clear WAL file {}", self.path.display()))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
