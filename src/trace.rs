//! Opt-in numeric timings. Input lines, arguments, paths and payloads never enter this API.
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Trace(Option<Arc<Inner>>);
struct Inner {
    start: Instant,
    writer: Mutex<BufWriter<File>>,
}
impl Trace {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("create new trace file {}", path.display()))?;
        Ok(Self(Some(Arc::new(Inner {
            start: Instant::now(),
            writer: Mutex::new(BufWriter::new(file)),
        }))))
    }
    pub fn enabled(&self) -> bool {
        self.0.is_some()
    }
    pub fn event(
        &self,
        event: &'static str,
        revision: Option<u64>,
        duration: Option<Duration>,
        count: Option<usize>,
    ) {
        let Some(inner) = &self.0 else {
            return;
        };
        let data = json!({"event":event,"elapsed_ms":inner.start.elapsed().as_secs_f64()*1000.0,
            "revision":revision,"duration_ms":duration.map(|d|d.as_secs_f64()*1000.0),"count":count});
        if let Ok(mut writer) = inner.writer.lock() {
            let _ = writeln!(writer, "{data}");
        }
    }
    pub fn flush(&self) {
        if let Some(inner) = &self.0 {
            if let Ok(mut writer) = inner.writer.lock() {
                let _ = writer.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_trace_has_no_file_and_enabled_events_are_numeric() {
        let disabled = Trace::open(None).unwrap();
        assert!(!disabled.enabled());
        disabled.event("query_sent", Some(7), None, None);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.jsonl");
        let trace = Trace::open(Some(&path)).unwrap();
        trace.event(
            "completion",
            Some(7),
            Some(Duration::from_micros(10)),
            Some(4),
        );
        trace.flush();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["event"], "completion");
        assert_eq!(value["revision"], 7);
        assert!(
            value
                .as_object()
                .unwrap()
                .iter()
                .all(|(key, v)| key == "event" || v.is_number() || v.is_null())
        );
        assert!(Trace::open(Some(&path)).is_err());
    }
}
