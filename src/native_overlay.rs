//! Internal client for the fenced companion interface. Acknowledgements never
//! enter the shell input stream; this module does not change user protocols.
use crate::pipe::NativeAckPipe;
use std::{
    io::{self, Write},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    ready: bool,
    stopped: bool,
    error: Option<String>,
    expected: u64,
    resolved: bool,
    visible: bool,
    retired: bool,
    closing: u64,
    closed: Option<bool>,
}

impl State {
    fn reply(&mut self, line: &str, owner: &str) -> io::Result<()> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid companion acknowledgement",
            )
        };
        if !self.ready {
            if line != format!("ready;4;{owner}") {
                return Err(invalid());
            }
            self.ready = true;
            return Ok(());
        }
        let fields: Vec<_> = line.split(';').collect();
        let id = fields
            .get(1)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|id| *id != 0)
            .ok_or_else(invalid)?;
        match fields.as_slice() {
            ["frame", _, "presented" | "rejected"] => {
                if id == self.expected {
                    self.resolved = true;
                    self.visible = fields[2] == "presented" && !self.retired;
                }
            }
            ["retired", _] => {
                if id == self.expected {
                    self.visible = false;
                    self.retired = true;
                }
            }
            ["closed", _, "restored" | "absent"] => {
                if id == self.closing {
                    self.closed = Some(fields[2] == "restored");
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct NativeOverlay {
    pipe: NativeAckPipe,
    shared: Shared,
    reader: Option<JoinHandle<()>>,
    generation: u64,
}

impl NativeOverlay {
    pub fn connect(output: &mut impl Write) -> io::Result<Self> {
        let pipe = NativeAckPipe::new()?;
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let read_pipe = pipe.clone();
        let state = shared.clone();
        let reader = thread::Builder::new()
            .name("Blueberry Terminal acknowledgements".into())
            .spawn(move || {
                let result = Self::receive(&read_pipe, &state);
                if let Ok(mut value) = state.0.lock() {
                    value.stopped = true;
                    value.visible = false;
                    value.error = result.err().map(|e| e.to_string());
                }
                state.1.notify_all();
            })?;
        let client = Self {
            pipe,
            shared,
            reader: Some(reader),
            generation: 0,
        };
        client.send(
            output,
            &format!("channel;{};{}", std::process::id(), client.pipe.owner()),
        )?;
        client.wait(Duration::from_millis(500), |s| s.ready)?;
        Ok(client)
    }

    fn receive(pipe: &NativeAckPipe, shared: &Shared) -> io::Result<()> {
        pipe.connect()?;
        let mut buffer = [0; 256];
        let mut line = Vec::with_capacity(192);
        loop {
            let count = pipe.read(&mut buffer)?;
            for byte in &buffer[..count] {
                if *byte == b'\n' {
                    let text = std::str::from_utf8(&line).map_err(io::Error::other)?;
                    let mut state = shared
                        .0
                        .lock()
                        .map_err(|_| io::Error::other("companion state poisoned"))?;
                    state.reply(text, pipe.owner())?;
                    shared.1.notify_all();
                    line.clear();
                } else {
                    if !(0x20..=0x7e).contains(byte) || line.len() >= 191 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "companion acknowledgement exceeds ASCII bound",
                        ));
                    }
                    line.push(*byte);
                }
            }
        }
    }

    fn send(&self, output: &mut impl Write, command: &str) -> io::Result<()> {
        output.write_all(format!("\x1b]7775;{command}\x07").as_bytes())?;
        output.flush()
    }

    fn wait(&self, timeout: Duration, done: impl Fn(&State) -> bool) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| io::Error::other("companion state poisoned"))?;
        loop {
            if state.stopped {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    state
                        .error
                        .clone()
                        .unwrap_or_else(|| "companion stopped".into()),
                ));
            }
            if done(&state) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "companion acknowledgement timed out",
                ));
            }
            state = self
                .shared
                .1
                .wait_timeout(state, remaining)
                .map_err(|_| io::Error::other("companion state poisoned"))?
                .0;
        }
    }

    pub fn paint(
        &mut self,
        output: &mut impl Write,
        lines: &[String],
        top: u16,
        left: u16,
        width: u16,
    ) -> io::Result<bool> {
        let length = lines.iter().try_fold(0usize, |length, line| {
            let next = length.checked_add(line.encode_utf16().take(65537).count() + 5)?;
            (next <= 65537).then_some(next)
        });
        if lines.is_empty() || lines.len() > 512 || width == 0 || width > 512 || length.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native menu exceeds frame bounds",
            ));
        }
        let length = length.unwrap() - 1;
        let text = lines
            .iter()
            .map(|line| format!("{line}\x1b[0m"))
            .collect::<Vec<_>>()
            .join("\n");
        // Clear an older generation before replacing it. If Terminal rejects
        // the next frame, no untracked old menu can remain visible after Esc.
        self.close(output)?;
        let mut hex = String::with_capacity(length * 4);
        let digits = b"0123456789abcdef";
        for unit in text.encode_utf16() {
            for shift in [12, 8, 4, 0] {
                hex.push(digits[((unit >> shift) & 15) as usize] as char);
            }
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("native generation exhausted"))?;
        let id = self.generation;
        {
            let mut state = self
                .shared
                .0
                .lock()
                .map_err(|_| io::Error::other("companion state poisoned"))?;
            if !state.ready || state.stopped {
                return Ok(false);
            }
            state.expected = id;
            state.resolved = false;
            state.visible = false;
            state.retired = false;
            state.closing = 0;
        }
        self.send(
            output,
            &format!(
                "native-frame;{};{id};{top};{left};{};{width};{hex}",
                self.pipe.owner(),
                lines.len()
            ),
        )?;
        if let Err(error) = self.wait(Duration::from_millis(250), |s| s.resolved) {
            let _ = self.close(output);
            return Err(error);
        }
        Ok(self
            .shared
            .0
            .lock()
            .map_err(|_| io::Error::other("companion state poisoned"))?
            .visible)
    }

    /// None means no frame needed retirement; Some(true) is the authoritative
    /// pre-key fence. A stale, absent or unconfirmed frame cannot authorize Tab.
    pub fn close(&self, output: &mut impl Write) -> io::Result<Option<bool>> {
        let (id, visible) = {
            let mut state = self
                .shared
                .0
                .lock()
                .map_err(|_| io::Error::other("companion state poisoned"))?;
            let id = state.expected;
            if id == 0 {
                return Ok(None);
            }
            let visible = state.visible && !state.stopped;
            state.expected = 0;
            state.visible = false;
            state.closing = id;
            state.closed = None;
            (id, visible)
        };
        self.send(output, &format!("native-close;{};{id}", self.pipe.owner()))?;
        self.wait(Duration::from_millis(250), |s| s.closed.is_some())?;
        let state = self
            .shared
            .0
            .lock()
            .map_err(|_| io::Error::other("companion state poisoned"))?;
        Ok(Some(visible && state.closed == Some(true)))
    }
}

impl Drop for NativeOverlay {
    fn drop(&mut self) {
        let _ = self.pipe.cancel();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NativeBuffer(Arc<Mutex<crate::native_observer::NativeObserver>>);
    impl Write for NativeBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap()
                .write(bytes)
                .map_err(|e| io::Error::other(format!("{e:#}")))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    #[ignore = "requires the frozen CI production-buffer DLL and digest"]
    fn production_terminal_round_trip_resize_and_owner_eof() {
        use crate::native_observer::NativeObserver;
        let environment =
            std::collections::BTreeMap::from([("BLUEBERRY_NATIVE_OVERLAY".into(), "4".into())]);
        let model = || NativeObserver::configured(&environment).unwrap().unwrap();
        let reference = model();
        let mut output = NativeBuffer(model());
        let history = "\x1b[38;2;17;123;239m\x1b[58;2;9;87;165m\x1b[4:3m\x1b]8;id=history;https://example.invalid/history\x1b\\历史🚀é-abcdefghijklmnopqrstuvwxyz-0123456789\x1b]8;;\x1b\\\x1b[0m\r\n".repeat(40);
        reference.lock().unwrap().write(history.as_bytes()).unwrap();
        output.write_all(history.as_bytes()).unwrap();
        let mut client = NativeOverlay::connect(&mut output).unwrap();
        let lines = vec!["\x1b[38;2;12;34;56mMENU中🚀é".to_owned()];
        for width in [40, 20, 60, 120] {
            assert!(client.paint(&mut output, &lines, 2, 3, 16).unwrap());
            assert!(
                output
                    .0
                    .lock()
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .contains("MENU")
            );
            reference.lock().unwrap().resize(12, width).unwrap();
            output.0.lock().unwrap().resize(12, width).unwrap();
            assert_eq!(client.close(&mut output).unwrap(), Some(false));
            assert_eq!(
                reference.lock().unwrap().snapshot().unwrap(),
                output.0.lock().unwrap().snapshot().unwrap()
            );
        }
        assert!(
            !client
                .paint(&mut output, &["\x1b[99mINVALID".to_owned()], 2, 3, 16)
                .unwrap()
        );
        assert_eq!(client.close(&mut output).unwrap(), Some(false));
        assert_eq!(
            reference.lock().unwrap().snapshot().unwrap(),
            output.0.lock().unwrap().snapshot().unwrap()
        );
        assert!(client.paint(&mut output, &lines, 2, 3, 16).unwrap());
        assert_eq!(client.close(&mut output).unwrap(), Some(true));
        assert_eq!(
            reference.lock().unwrap().snapshot().unwrap(),
            output.0.lock().unwrap().snapshot().unwrap()
        );
        assert!(client.paint(&mut output, &lines, 2, 3, 16).unwrap());
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if reference.lock().unwrap().snapshot().unwrap()
                == output.0.lock().unwrap().snapshot().unwrap()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owner EOF did not restore the native buffer"
            );
            thread::yield_now();
        }
    }

    #[derive(Default)]
    struct TerminalMock {
        buffered: Vec<u8>,
        client: Option<std::fs::File>,
        absent: bool,
    }

    impl Write for TerminalMock {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.buffered.extend_from_slice(bytes);
            while let Some(end) = self.buffered.iter().position(|v| *v == 7) {
                let command: Vec<_> = self.buffered.drain(..=end).collect();
                let command = std::str::from_utf8(&command[7..command.len() - 1]).unwrap();
                let fields: Vec<_> = command.split(';').collect();
                let reply =
                    match fields.as_slice() {
                        ["channel", _, owner] => {
                            self.client =
                                Some(std::fs::OpenOptions::new().read(true).write(true).open(
                                    format!(r"\\.\pipe\Blueberry.TerminalOverlay.{owner}"),
                                )?);
                            format!("ready;4;{owner}\n")
                        }
                        ["native-frame", _, id, _, _, _, _, hex] => {
                            assert!(!hex.is_empty() && hex.len() % 4 == 0);
                            format!("frame;{id};presented\n")
                        }
                        ["native-close", _, id] => format!(
                            "closed;{id};{}\n",
                            if self.absent { "absent" } else { "restored" }
                        ),
                        _ => return Err(io::Error::other("unexpected mock command")),
                    };
                self.client.as_mut().unwrap().write_all(reply.as_bytes())?;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn real_pipe_confirmed_submission_requires_authoritative_close() {
        let mut terminal = TerminalMock::default();
        let mut client = NativeOverlay::connect(&mut terminal).unwrap();
        let lines = vec!["\x1b[38;2;12;34;56m中🚀é".to_owned()];
        assert!(client.paint(&mut terminal, &lines, 2, 3, 16).unwrap());
        let generation = client.generation;
        assert!(
            client
                .paint(&mut terminal, &["x".repeat(65537)], 2, 3, 16)
                .is_err()
        );
        assert_eq!(client.generation, generation);
        assert_eq!(client.close(&mut terminal).unwrap(), Some(true));
        assert_eq!(client.close(&mut terminal).unwrap(), None);
        assert!(client.paint(&mut terminal, &lines, 2, 3, 16).unwrap());
        terminal.absent = true;
        assert_eq!(client.close(&mut terminal).unwrap(), Some(false));
        // Dropping while the reader is blocked must cancel and join it.
        let start = Instant::now();
        drop(client);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn handshake_timeout_and_eof_do_not_leave_a_reader() {
        assert!(NativeOverlay::connect(&mut Vec::new()).is_err());
        let mut terminal = TerminalMock::default();
        let mut client = NativeOverlay::connect(&mut terminal).unwrap();
        drop(terminal.client.take());
        assert!(client.wait(Duration::from_secs(2), |_| false).is_err());
        assert!(
            !client
                .paint(&mut Vec::new(), &["menu".to_owned()], 0, 0, 10)
                .unwrap()
        );
    }

    #[test]
    fn owner_and_fenced_version_are_required() {
        for line in ["ready;3;owner", "ready;4;other", "frame;1;presented"] {
            assert!(State::default().reply(line, "owner").is_err());
        }
        let mut state = State::default();
        state.reply("ready;4;owner", "owner").unwrap();
        assert!(state.reply("ready;4;owner", "owner").is_err());
    }

    #[test]
    fn stale_and_retired_frames_cannot_be_resurrected() {
        let mut state = State {
            ready: true,
            expected: 2,
            ..Default::default()
        };
        state.reply("frame;1;presented", "owner").unwrap();
        assert!(!state.resolved && !state.visible);
        state.reply("retired;2", "owner").unwrap();
        state.reply("frame;2;presented", "owner").unwrap();
        assert!(state.resolved && !state.visible);
        state.closing = 2;
        state.reply("closed;1;restored", "owner").unwrap();
        assert!(state.closed.is_none());
        state.reply("closed;2;absent", "owner").unwrap();
        assert_eq!(state.closed, Some(false));
    }
}
