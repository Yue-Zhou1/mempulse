use anyhow::{Context, Result};
use event_log::EventEnvelope;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const DEFAULT_SEGMENT_MAX_BYTES: u64 = 64 * 1024 * 1024;
const HIGH_THROUGHPUT_SEGMENT_MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct StorageWal {
    path: PathBuf,
    segment_max_bytes: u64,
}

impl StorageWal {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        Self::with_segment_size(path, DEFAULT_SEGMENT_MAX_BYTES)
    }

    pub fn with_segment_size(path: impl Into<PathBuf>, segment_max_bytes: u64) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create WAL parent directory {}", parent.display()))?;
        }
        Ok(Self {
            path,
            segment_max_bytes: segment_max_bytes.max(1),
        })
    }

    pub fn high_throughput(path: impl Into<PathBuf>) -> Result<Self> {
        Self::with_segment_size(path, HIGH_THROUGHPUT_SEGMENT_MAX_BYTES)
    }

    pub fn segment_max_bytes(&self) -> u64 {
        self.segment_max_bytes
    }

    pub fn append_event(&self, event: &EventEnvelope) -> Result<()> {
        let segment_path = self.active_segment_path_for_append()?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)
            .with_context(|| format!("open WAL segment {} for append", segment_path.display()))?;
        let encoded = serde_json::to_string(event).context("serialize WAL event")?;
        writeln!(file, "{encoded}")
            .with_context(|| format!("append WAL event to segment {}", segment_path.display()))?;
        Ok(())
    }

    pub fn recover_events(&self) -> Result<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        let segments = self.segment_paths()?;
        for (_, path) in segments {
            events.extend(read_events_from_path(&path)?);
        }

        if self.path.exists() {
            events.extend(read_events_from_path(&self.path)?);
        }

        events.sort_by_key(|event| event.seq_id);
        Ok(events)
    }

    pub fn scan(&self, from_seq_id: u64, limit: usize) -> Result<Vec<EventEnvelope>> {
        let limit = limit.max(1);
        let events = self.recover_events()?;
        Ok(events
            .into_iter()
            .filter(|event| event.seq_id > from_seq_id)
            .take(limit)
            .collect())
    }

    pub fn clear(&self) -> Result<()> {
        let segments = self.segment_paths()?;
        for (_, path) in segments {
            fs::remove_file(&path)
                .with_context(|| format!("remove WAL segment {}", path.display()))?;
        }

        if self.path.exists() {
            fs::write(&self.path, b"")
                .with_context(|| format!("clear WAL file {}", self.path.display()))?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn active_segment_path_for_append(&self) -> Result<PathBuf> {
        let segments = self.segment_paths()?;
        if let Some((id, path)) = segments.last() {
            let len = fs::metadata(path)
                .with_context(|| format!("stat WAL segment {}", path.display()))?
                .len();
            if len < self.segment_max_bytes {
                return Ok(path.clone());
            }
            return Ok(self.segment_path(id.saturating_add(1))?);
        }

        self.segment_path(0)
    }

    fn segment_paths(&self) -> Result<Vec<(u64, PathBuf)>> {
        let parent = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let prefix = format!("{}.seg.", self.base_name());
        let mut segments = Vec::new();

        for entry in fs::read_dir(&parent)
            .with_context(|| format!("read WAL directory {}", parent.display()))?
        {
            let entry =
                entry.with_context(|| format!("read WAL directory entry {}", parent.display()))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) {
                continue;
            }
            let Some(id_raw) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Ok(id) = id_raw.parse::<u64>() else {
                continue;
            };
            segments.push((id, path));
        }

        segments.sort_by_key(|(id, _)| *id);
        Ok(segments)
    }

    fn segment_path(&self, id: u64) -> Result<PathBuf> {
        let parent = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let base = self.base_name();
        Ok(parent.join(format!("{base}.seg.{id:020}")))
    }

    fn base_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| "events".to_owned())
    }
}

fn read_events_from_path(path: &Path) -> Result<Vec<EventEnvelope>> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("open WAL file {} for recovery", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line.context("read WAL line")?;
        if line.trim().is_empty() {
            continue;
        }
        let event: EventEnvelope = serde_json::from_str(&line).context("decode WAL event line")?;
        events.push(event);
    }
    Ok(events)
}
