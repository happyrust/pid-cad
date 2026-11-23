//! Runtime observability metrics and structured single-line JSON file logging (#39).
//!
//! All counters are lock-free or guarded by short-lived mutexes; structured log
//! lines never contain user credentials or secrets.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

/// Per-process runtime metrics collected by the collaboration server.
pub struct Metrics {
    started_at: i64,
    applied_ops_total: AtomicU64,
    denied_writes_total: AtomicU64,
    /// Per-drawing timestamps (unix seconds) of applied ops in the trailing 60s.
    recent_ops: Mutex<HashMap<String, VecDeque<i64>>>,
    /// Per-drawing count of denied writes since server startup.
    denied_by_drawing: Mutex<HashMap<String, u64>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new(ocs_collab::now_unix())
    }
}

impl Metrics {
    pub fn new(started_at: i64) -> Self {
        Self {
            started_at,
            applied_ops_total: AtomicU64::new(0),
            denied_writes_total: AtomicU64::new(0),
            recent_ops: Mutex::new(HashMap::new()),
            denied_by_drawing: Mutex::new(HashMap::new()),
        }
    }

    pub fn started_at(&self) -> i64 {
        self.started_at
    }

    /// Records one applied op on `drawing_id` at unix second `now`.
    pub fn record_applied_op(&self, drawing_id: &str, now: i64) {
        self.applied_ops_total.fetch_add(1, Ordering::Relaxed);
        let mut map = self.recent_ops.lock().expect("metrics recent_ops mutex");
        let queue = map.entry(drawing_id.to_owned()).or_default();
        queue.push_back(now);
        let cutoff = now - 60;
        while queue.front().is_some_and(|&ts| ts < cutoff) {
            queue.pop_front();
        }
    }

    /// Records one denied write on `drawing_id`.
    pub fn record_denied_write(&self, drawing_id: &str) {
        self.denied_writes_total.fetch_add(1, Ordering::Relaxed);
        let mut map = self
            .denied_by_drawing
            .lock()
            .expect("metrics denied_by_drawing mutex");
        *map.entry(drawing_id.to_owned()).or_insert(0) += 1;
    }

    pub fn applied_ops_total(&self) -> u64 {
        self.applied_ops_total.load(Ordering::Relaxed)
    }

    pub fn denied_writes_total(&self) -> u64 {
        self.denied_writes_total.load(Ordering::Relaxed)
    }

    /// Per-drawing applied ops within the last 60 seconds relative to `now`.
    pub fn ops_per_minute_by_drawing(&self, now: i64) -> HashMap<String, u64> {
        let mut map = self.recent_ops.lock().expect("metrics recent_ops mutex");
        let cutoff = now - 60;
        let mut out = HashMap::new();
        for (drawing_id, queue) in map.iter_mut() {
            while queue.front().is_some_and(|&ts| ts < cutoff) {
                queue.pop_front();
            }
            if !queue.is_empty() {
                out.insert(drawing_id.clone(), queue.len() as u64);
            }
        }
        out
    }

    /// Per-drawing denied write totals since startup.
    pub fn denied_writes_by_drawing(&self) -> HashMap<String, u64> {
        self.denied_by_drawing
            .lock()
            .expect("metrics denied_by_drawing mutex")
            .clone()
    }
}

/// Single-line structured JSON logger with optional file output (`--log-file`, #39).
pub struct StructuredLogger {
    file: Mutex<Option<BufWriter<File>>>,
}

impl Default for StructuredLogger {
    fn default() -> Self {
        Self {
            file: Mutex::new(None),
        }
    }
}

impl StructuredLogger {
    /// Opens (creating/appending) a file for single-line JSON log output.
    pub fn with_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("创建日志目录 {} 失败：{err}", parent.display()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("打开日志文件 {} 失败：{err}", path.display()))?;
        Ok(Self {
            file: Mutex::new(Some(BufWriter::new(file))),
        })
    }

    /// Attaches a file sink to an existing logger.
    pub fn set_file(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("创建日志目录 {} 失败：{err}", parent.display()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("打开日志文件 {} 失败：{err}", path.display()))?;
        *self.file.lock().expect("logger mutex") = Some(BufWriter::new(file));
        Ok(())
    }

    /// Writes one single-line JSON record (`{"ts":...,"level":...,"event":...}`)
    /// to the configured log file (if any). Callers must NEVER pass credentials.
    pub fn emit(&self, level: &str, event: &str, fields: Value) {
        let mut obj = match fields {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        // Defensive scrub: never allow credential fields into structured logs.
        obj.remove("credential");
        obj.remove("credential_hash");
        obj.remove("password");
        obj.insert("ts".to_owned(), json!(ocs_collab::now_unix()));
        obj.insert("level".to_owned(), json!(level));
        obj.insert("event".to_owned(), json!(event));
        let line = Value::Object(obj).to_string();

        if let Some(writer) = self.file.lock().expect("logger mutex").as_mut() {
            let _ = writeln!(writer, "{line}");
            let _ = writer.flush();
        }
    }
}
