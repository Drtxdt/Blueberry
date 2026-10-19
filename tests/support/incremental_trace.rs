use anyhow::Result;
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Copy, Default)]
pub struct State {
    pub keys: usize,
    pub key_revision: Option<u64>,
    pub painted_revision: u64,
    pub applied: usize,
}

/// Consume a bounded amount of new trace data between terminal pumps. Keep
/// partial lines until their newline arrives; never reparse the entire log.
pub struct Trace {
    file: File,
    pending: Vec<u8>,
    pub state: State,
}

impl Trace {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            pending: Vec::new(),
            state: State::default(),
        })
    }

    pub fn poll(&mut self) -> Result<bool> {
        let mut bytes = [0; 16 * 1024];
        let count = self.file.read(&mut bytes)?;
        self.pending.extend_from_slice(&bytes[..count]);
        if let Some(end) = self.pending.iter().rposition(|byte| *byte == b'\n') {
            for line in self.pending[..=end].split(|byte| *byte == b'\n') {
                if line.is_empty() {
                    continue;
                }
                let event: serde_json::Value = serde_json::from_slice(line)?;
                match event["event"].as_str() {
                    Some("direct_ui_key") => {
                        self.state.keys += 1;
                        self.state.key_revision = event["revision"].as_u64();
                    }
                    Some("editor_menu_written") => {
                        self.state.painted_revision = event["revision"].as_u64().unwrap_or(0);
                    }
                    Some("editor_edit_applied") => self.state.applied += 1,
                    _ => {}
                }
            }
            self.pending.drain(..=end);
        }
        anyhow::ensure!(
            self.pending.len() <= 4 * 1024 * 1024,
            "unterminated trace record"
        );
        Ok(count < bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn partial_records_and_repeated_polls_preserve_event_counts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("trace.jsonl");
        let mut writer = File::create(&path)?;
        let mut trace = Trace::open(&path)?;
        writer.write_all(br#"{"event":"direct_ui_key","revision":7}"#)?;
        assert!(trace.poll()?);
        assert_eq!(trace.state.keys, 0);
        writer.write_all(b"\n{\"event\":\"editor_menu_written\",\"revision\":7}\n")?;
        assert!(trace.poll()?);
        assert_eq!(trace.state.keys, 1);
        assert_eq!(trace.state.key_revision, Some(7));
        assert_eq!(trace.state.painted_revision, 7);
        assert!(trace.poll()?);
        assert_eq!(trace.state.keys, 1);
        writer.write_all(b"{\"event\":\"editor_edit_applied\"}\n")?;
        trace.poll()?;
        assert_eq!(trace.state.applied, 1);
        Ok(())
    }
}
