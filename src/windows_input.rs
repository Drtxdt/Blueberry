//! Windows console input with bracketed-paste support.
//!
//! With `ENABLE_VIRTUAL_TERMINAL_INPUT` enabled, ConPTY exposes terminal
//! bytes as `KEY_EVENT_RECORD`s with `wVirtualKeyCode == 0`. The regular
//! crossterm Windows reader does not retain those bytes, so this module owns
//! the native `CONIN$` queue and forwards the byte stream to the crate's
//! incremental VT parser. Native key, mouse, resize and focus records keep
//! their existing crossterm-compatible representation.

#![cfg(windows)]

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{
    collections::VecDeque,
    io,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Console::{
            CONSOLE_SCREEN_BUFFER_INFO, FOCUS_EVENT, GetConsoleScreenBufferInfo,
            GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD, MOUSE_EVENT,
            MOUSE_EVENT_RECORD, ReadConsoleInputW, WINDOW_BUFFER_SIZE_EVENT,
        },
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
        Ole::CF_UNICODETEXT,
    },
};

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const MAX_PASTE_BYTES: usize = 1024 * 1024;
const MAX_VT_BUFFER_BYTES: usize = 4096;
const MAX_NATIVE_BATCH_RECORDS: usize = 65_536;

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0d;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1b;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_INSERT: u16 = 0x2d;
const VK_DELETE: u16 = 0x2e;
const VK_SPACE: u16 = 0x20;
const VK_NUMPAD0: u16 = 0x60;
const VK_NUMPAD9: u16 = 0x69;
const VK_F1: u16 = 0x70;
const VK_F24: u16 = 0x87;

const SHIFT_PRESSED: u32 = 0x0010;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_ALT_PRESSED: u32 = 0x0001;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;

const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;
const RIGHTMOST_BUTTON_PRESSED: u32 = 0x0002;
const FROM_LEFT_2ND_BUTTON_PRESSED: u32 = 0x0004;
const FROM_LEFT_3RD_BUTTON_PRESSED: u32 = 0x0008;
const FROM_LEFT_4TH_BUTTON_PRESSED: u32 = 0x0010;
const MOUSE_MOVED: u32 = 0x0001;
const DOUBLE_CLICK: u32 = 0x0002;
const MOUSE_WHEELED: u32 = 0x0004;
const MOUSE_HWHEELED: u32 = 0x0008;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputTransport {
    /// No input record has identified the terminal's wire format yet.
    /// Ordinary VT and Win32 CSI_ records share the ESC-prefixed start.
    Unknown,
    /// The console is delivering terminal bytes directly as VK0 records.
    Direct,
    /// The console is wrapping each key record in a Win32 CSI_ envelope.
    Win32Envelope,
}

// The crate already enables Win32.System.Console. crossterm opens these
// devices itself, but crossterm_winapi is a transitive implementation detail;
// keep the production reader's dependency surface stable with two small FFI
// declarations instead of adding another runtime or a second Windows crate.
unsafe extern "system" {
    fn CreateFileW(
        lp_file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: HANDLE,
    ) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> i32;
}

#[derive(Clone, Copy)]
struct KeyRecord {
    down: bool,
    repeat: u16,
    virtual_key: u16,
    scan: u16,
    unicode: u16,
    control_state: u32,
}

impl From<KEY_EVENT_RECORD> for KeyRecord {
    fn from(record: KEY_EVENT_RECORD) -> Self {
        Self {
            down: record.bKeyDown != 0,
            repeat: record.wRepeatCount,
            virtual_key: record.wVirtualKeyCode,
            scan: record.wVirtualScanCode,
            unicode: unsafe { record.uChar.UnicodeChar },
            control_state: record.dwControlKeyState,
        }
    }
}

#[derive(Default)]
struct MouseButtons {
    left: bool,
    right: bool,
    middle: bool,
}

struct PasteState {
    text: Vec<u8>,
    end_prefix: Vec<u8>,
    overflowed: bool,
}

impl PasteState {
    fn new() -> Self {
        Self {
            text: Vec::new(),
            end_prefix: Vec::new(),
            overflowed: false,
        }
    }

    fn push_text(&mut self, byte: u8) {
        if self.text.len() < MAX_PASTE_BYTES {
            self.text.push(byte);
        } else {
            self.overflowed = true;
        }
    }

    /// Feed one UTF-8 byte. The return value is the completed paste, if the
    /// end marker was just consumed. The end marker itself is never appended
    /// to the text, including when it crosses native record batches.
    fn feed(&mut self, byte: u8) -> Option<(Vec<u8>, bool)> {
        loop {
            let index = self.end_prefix.len();
            if byte == PASTE_END[index] {
                self.end_prefix.push(byte);
                if self.end_prefix.len() == PASTE_END.len() {
                    self.end_prefix.clear();
                    return Some((std::mem::take(&mut self.text), self.overflowed));
                }
                return None;
            }
            if self.end_prefix.is_empty() {
                self.push_text(byte);
                return None;
            }
            // A partial end marker was ordinary paste text. Reprocess the
            // current byte in case it starts a new marker.
            let held = std::mem::take(&mut self.end_prefix);
            for byte in held {
                self.push_text(byte);
            }
        }
    }
}

/// Set when a paste exceeded the bounded payload size and was consumed up to
/// its closing marker. The host can turn this into its normal diagnostic
/// event without ever forwarding a partial paste to the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteRejection {
    TooLarge,
}

/// A Windows console reader that emits crossterm events and coalesces
/// bracketed paste into one `Event::Paste`.
///
/// The process must enable `ENABLE_VIRTUAL_TERMINAL_INPUT` in its raw input
/// mode before constructing this reader. `Reader` does not change or restore
/// console mode; the host's existing raw-mode guard remains the owner of that
/// lifecycle.
pub struct Reader {
    input: HANDLE,
    output: Option<HANDLE>,
    records: VecDeque<INPUT_RECORD>,
    events: VecDeque<Event>,
    /// The outer bytes currently being decoded. In direct mode these are
    /// terminal bytes; in Win32 input mode they are CSI_ records describing
    /// one native key event.
    wire_buffer: Vec<u8>,
    /// Terminal bytes reconstructed from a Win32 CSI_ record's VK0 payload.
    /// This must remain separate from wire_buffer: an inner ESC prefix can
    /// span several outer records.
    payload_buffer: Vec<u8>,
    /// Bytes beginning with ESC that may be the paste-start marker in the
    /// reconstructed terminal stream. Holding this prefix also lets the VT
    /// parser see a complete CSI/SS3 sequence after a marker mismatch.
    payload_prefix: Vec<u8>,
    transport: InputTransport,
    paste: Option<PasteState>,
    /// Bytes currently owned by crate::vt_input::parse_event. The parser is
    /// incremental and returns one event at a time; therefore this buffer is
    /// cleared only when it reports a complete event or ignored response.
    ///
    /// The two buffers have independent overflow states. A malformed or
    /// hostile control sequence must never make the host exit, and an
    /// incomplete sequence must not grow without bound while waiting for its
    /// final byte.
    wire_discard_until_final: bool,
    payload_discard_until_final: bool,
    /// UTF-16 state for direct/raw terminal code units or the outer CSI_
    /// byte stream. This must not be shared with a decoded inner record:
    /// the next outer envelope starts with ASCII ESC and must not complete
    /// an inner high surrogate.
    wire_surrogate: Option<u16>,
    /// UTF-16 state for the logical terminal payload reconstructed from a
    /// Win32 CSI_ envelope (including bracketed-paste text).
    payload_surrogate: Option<u16>,
    /// UTF-16 state for ordinary native key mapping outside the VT payload.
    native_text_surrogate: Option<u16>,
    mouse_buttons: MouseButtons,
    paste_rejection: Option<PasteRejection>,
    unmarked_paste: Option<PendingUnmarkedPaste>,
}

struct PendingUnmarkedPaste {
    target: String,
    events: Vec<Event>,
}

impl Reader {
    /// Open the current console input buffer (`CONIN$`).
    pub fn new() -> io::Result<Self> {
        let input = open_console_device("CONIN$")?;
        let output = open_console_device("CONOUT$").ok();
        Ok(Self {
            input,
            output,
            records: VecDeque::new(),
            events: VecDeque::new(),
            wire_buffer: Vec::new(),
            payload_buffer: Vec::new(),
            payload_prefix: Vec::new(),
            transport: InputTransport::Unknown,
            paste: None,
            wire_discard_until_final: false,
            payload_discard_until_final: false,
            wire_surrogate: None,
            payload_surrogate: None,
            native_text_surrogate: None,
            mouse_buttons: MouseButtons::default(),
            paste_rejection: None,
            unmarked_paste: None,
        })
    }

    /// Return and clear the most recent bounded-paste rejection.
    pub fn take_paste_rejection(&mut self) -> Option<PasteRejection> {
        self.paste_rejection.take()
    }

    /// Read a batch, blocking only when no event, rejection, or queued native
    /// record can make progress. After at least one event is ready, draining
    /// is limited to records already present in the native queue; this avoids
    /// turning a modifier/key-up-only tail into a second blocking read.
    pub fn read_batch(&mut self, max_events: usize) -> io::Result<Vec<Event>> {
        if max_events == 0 {
            return Ok(Vec::new());
        }
        let mut events = Vec::with_capacity(max_events);
        loop {
            while events.len() < max_events {
                if let Some(event) = self.events.pop_front() {
                    events.push(event);
                    continue;
                }
                if self.paste_rejection.is_some() {
                    return Ok(self.finish_unmarked_paste(std::mem::take(&mut events)));
                }
                if let Some(record) = self.records.pop_front() {
                    self.consume_record(record)?;
                    continue;
                }
                break;
            }

            if events.len() == max_events || self.paste_rejection.is_some() {
                let ready = self.finish_unmarked_paste(std::mem::take(&mut events));
                if ready.is_empty() {
                    continue;
                }
                return Ok(ready);
            }
            if self.available_records()? != 0 {
                self.read_records_blocking()?;
                continue;
            }
            if !events.is_empty() {
                let ready = self.finish_unmarked_paste(std::mem::take(&mut events));
                if ready.is_empty() {
                    continue;
                }
                return Ok(ready);
            }
            // There is no safe timeout for an ESC prefix: it may be the
            // beginning of the paste marker or a split CSI/SS3 sequence.
            // Keep waiting for the next native record rather than guessing
            // from a temporarily empty queue and replaying an executable key.
            self.read_records_blocking()?;
        }
    }

    fn finish_unmarked_paste(&mut self, events: Vec<Event>) -> Vec<Event> {
        if let Some(mut pending) = self.unmarked_paste.take() {
            pending.events.extend(events);
            let (end, text) = text_event_run(&pending.events, 0);
            if text == pending.target {
                let mut output = vec![Event::Paste(text)];
                output.extend(pending.events.drain(end..));
                return output;
            }
            if pending.target.starts_with(&text) && text.len() < pending.target.len() {
                self.unmarked_paste = Some(pending);
                return Vec::new();
            }
            return coalesce_unmarked_multiline_events(pending.events);
        }

        let events = coalesce_unmarked_multiline_events(events);
        let start = trailing_text_run_start(&events);
        let (_, text) = text_event_run(&events, start);
        let injected_records = !events[start..]
            .iter()
            .any(|event| matches!(event, Event::Key(key) if key.kind == KeyEventKind::Release));
        #[cfg(debug_assertions)]
        let deterministic_probe = std::env::var_os("BLUEBERRY_TEST_CLIPBOARD_TEXT").is_some();
        #[cfg(not(debug_assertions))]
        let deterministic_probe = false;
        if text.is_empty() || !(text.contains('\n') || injected_records || deterministic_probe) {
            return events;
        }
        let Some(target) = clipboard_multiline_text() else {
            return events;
        };
        if text.len() < target.len() && target.starts_with(&text) {
            let mut prefix = events[..start].to_vec();
            self.unmarked_paste = Some(PendingUnmarkedPaste {
                target,
                events: events[start..].to_vec(),
            });
            prefix.shrink_to_fit();
            return prefix;
        }
        events
    }

    fn read_records_blocking(&mut self) -> io::Result<()> {
        let mut drained = Vec::new();
        let mut complete_batch = true;
        loop {
            let remaining = MAX_NATIVE_BATCH_RECORDS.saturating_sub(drained.len());
            if remaining == 0 {
                complete_batch = false;
                break;
            }
            let mut records = vec![INPUT_RECORD::default(); remaining.min(256)];
            let mut count = 0u32;
            if unsafe {
                ReadConsoleInputW(
                    self.input,
                    records.as_mut_ptr(),
                    records.len() as u32,
                    &mut count,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            drained.extend(records.into_iter().take(count as usize));
            if self.available_records()? == 0 {
                break;
            }
        }
        // Windows Terminal normally brackets a right-click paste. Some
        // console paths instead enqueue the clipboard as one key-down-only
        // text burst. A multiline burst must remain one PSReadLine edit: if
        // its CR/LF records are exposed as Enter keys, the host can submit the
        // first physical line before the remainder reaches the child.
        //
        // This fallback is structural rather than time based. Physical input
        // contains virtual-key press/release pairs and therefore remains in
        // the ordinary key path.
        if complete_batch && let Some(text) = unmarked_paste_from_input_records(&drained) {
            self.events.push_back(Event::Paste(text));
            return Ok(());
        }
        self.records.extend(drained);
        Ok(())
    }

    fn available_records(&self) -> io::Result<u32> {
        let mut count = 0u32;
        if unsafe { GetNumberOfConsoleInputEvents(self.input, &mut count) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count)
    }

    fn consume_record(&mut self, record: INPUT_RECORD) -> io::Result<()> {
        match u32::from(record.EventType) {
            KEY_EVENT => self.consume_key(KeyRecord::from(unsafe { record.Event.KeyEvent })),
            MOUSE_EVENT => {
                if let Some(event) = self.mouse_event(unsafe { record.Event.MouseEvent }) {
                    self.events.push_back(event);
                }
                Ok(())
            }
            WINDOW_BUFFER_SIZE_EVENT => {
                let size = unsafe { record.Event.WindowBufferSizeEvent };
                self.events.push_back(Event::Resize(
                    size.dwSize.X.max(0) as u16,
                    size.dwSize.Y.max(0) as u16,
                ));
                Ok(())
            }
            FOCUS_EVENT => {
                let focus = unsafe { record.Event.FocusEvent };
                self.events.push_back(if focus.bSetFocus != 0 {
                    Event::FocusGained
                } else {
                    Event::FocusLost
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn consume_key(&mut self, key: KeyRecord) -> io::Result<()> {
        // With VT input enabled, a physical Escape arrives as a raw ESC byte
        // followed by a native VK_ESCAPE key-up record. That release is the
        // console's unambiguous boundary for a lone physical Escape; use it
        // to complete the parser with input_available=false. A raw ESC byte
        // from an ANSI sequence has no such release and remains buffered
        // until its sequence supplies the next byte.
        if is_physical_escape_release(key)
            && self.transport == InputTransport::Unknown
            && self.wire_buffer == [PASTE_START[0]]
        {
            self.transport = InputTransport::Direct;
            self.flush_wire_buffer(false)?;
            return Ok(());
        }
        if is_physical_escape_release(key)
            && self.transport == InputTransport::Direct
            && self.payload_prefix == [PASTE_START[0]]
        {
            self.flush_payload_prefix(false)?;
            return Ok(());
        }

        // Once a direct VT stream has entered bracketed paste, every logical
        // key record up to the end marker is payload. In particular, Windows
        // may attach a non-zero virtual key to printable ASCII; emitting that
        // record as a shell key would leak paste text before the marker is
        // consumed. Key-up records are bookkeeping noise inside a paste.
        if self.transport == InputTransport::Direct && self.paste.is_some() {
            if is_paste_alt_code_unit(key) {
                return self.consume_paste_code_unit(key);
            }
            if !key.down {
                return Ok(());
            }
            if is_paste_modifier_noise(key) {
                return Ok(());
            }
            return self.consume_wire_code_unit(key);
        }

        if key.down && is_vt_character(&key) {
            self.consume_wire_code_unit(key)?;
            return Ok(());
        }

        if !key.down
            && ((self.transport == InputTransport::Direct && !self.payload_prefix.is_empty())
                || !self.wire_buffer.is_empty())
        {
            // A key-up belonging to the record immediately before a split
            // VT prefix may be interleaved with the prefix bytes. It has no
            // editing effect and must not terminate marker matching.
            return Ok(());
        }
        if !self.wire_buffer.is_empty() {
            self.flush_wire_buffer(true)?;
        }
        // A VT key-up with vk=0 is the duplicate release generated for a raw
        // code unit. crossterm's parser would otherwise expose it as a second
        // key event; dropping it retains the existing press semantics.
        if !key.down && is_vt_character(&key) {
            return Ok(());
        }
        self.push_key_events(key);
        Ok(())
    }

    fn consume_wire_code_unit(&mut self, key: KeyRecord) -> io::Result<()> {
        let bytes = match self.transport {
            InputTransport::Direct => utf8_bytes(key.unicode, &mut self.payload_surrogate),
            InputTransport::Unknown | InputTransport::Win32Envelope => {
                utf8_bytes(key.unicode, &mut self.wire_surrogate)
            }
        };
        for _ in 0..key_repeat_count(key) {
            for byte in bytes.iter().copied() {
                match self.transport {
                    InputTransport::Direct => self.consume_payload_byte(byte, true)?,
                    InputTransport::Unknown | InputTransport::Win32Envelope => {
                        self.consume_wire_byte(byte, true)?
                    }
                }
            }
        }
        Ok(())
    }

    fn consume_paste_byte(&mut self, byte: u8) -> io::Result<()> {
        let Some(mut state) = self.paste.take() else {
            // A repeated end-marker code unit can leave additional logical
            // presses in the same INPUT_RECORD. Those presses are ordinary
            // input after the paste and must continue through the VT parser.
            return self.consume_payload_byte(byte, true);
        };
        if let Some((text, overflowed)) = state.feed(byte) {
            if overflowed {
                self.paste_rejection = Some(PasteRejection::TooLarge);
            } else if let Ok(text) = String::from_utf8(text) {
                self.events.push_back(Event::Paste(text));
            } else {
                // utf8_bytes always emits valid UTF-8 for a well-formed
                // UTF-16 stream. Keep malformed data out of the shell if a
                // hostile console record violates that invariant.
                self.paste_rejection = Some(PasteRejection::TooLarge);
            }
        } else {
            self.paste = Some(state);
        }
        Ok(())
    }

    fn consume_wire_byte(&mut self, byte: u8, input_available: bool) -> io::Result<()> {
        self.feed_wire_byte(byte, input_available)
    }

    fn feed_wire_byte(&mut self, byte: u8, input_available: bool) -> io::Result<()> {
        if self.wire_discard_until_final {
            if is_vt_final_byte(byte) {
                self.wire_discard_until_final = false;
            }
            return Ok(());
        }
        if self.wire_buffer.len() >= MAX_VT_BUFFER_BYTES {
            self.wire_buffer.clear();
            self.wire_discard_until_final = !is_vt_final_byte(byte);
            return Ok(());
        }
        self.wire_buffer.push(byte);
        self.feed_wire_parser(input_available)
    }

    fn flush_wire_buffer(&mut self, input_available: bool) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.wire_buffer);
        for (index, byte) in bytes.iter().copied().enumerate() {
            let more = input_available || index + 1 < bytes.len();
            self.feed_wire_byte(byte, more)?;
        }
        Ok(())
    }

    fn consume_payload_byte(&mut self, byte: u8, input_available: bool) -> io::Result<()> {
        if self.paste.is_some() {
            return self.consume_paste_byte(byte);
        }
        if self.payload_prefix.is_empty() {
            if byte == PASTE_START[0] {
                self.payload_prefix.push(byte);
                return Ok(());
            }
            return self.feed_payload_parser(byte, input_available);
        }

        let index = self.payload_prefix.len();
        if index < PASTE_START.len() && byte == PASTE_START[index] {
            self.payload_prefix.push(byte);
            if self.payload_prefix.len() == PASTE_START.len() {
                self.payload_prefix.clear();
                self.paste = Some(PasteState::new());
            }
            return Ok(());
        }

        // The prefix is not a paste marker. Feed its bytes and this byte to
        // the inner incremental VT parser in order; a CSI/SS3 sequence is
        // then reconstructed without any special case in the native reader.
        // Two adjacent ESC bytes are two physical Escape keys. Finalize the
        // first one before holding the second; otherwise the parser retains
        // the first ESC as an Alt prefix and the next ordinary character is
        // reported as Alt+character.
        let lone_escape_before_escape =
            self.payload_prefix == [PASTE_START[0]] && byte == PASTE_START[0];
        self.flush_payload_prefix(!lone_escape_before_escape)?;
        self.consume_payload_byte(byte, input_available)
    }

    fn flush_payload_prefix(&mut self, input_available: bool) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.payload_prefix);
        for (index, byte) in bytes.iter().copied().enumerate() {
            let more = input_available || index + 1 < bytes.len();
            self.feed_payload_parser(byte, more)?;
        }
        Ok(())
    }

    fn feed_wire_parser(&mut self, input_available: bool) -> io::Result<()> {
        if self.transport == InputTransport::Unknown && self.wire_buffer == PASTE_START {
            self.wire_buffer.clear();
            self.transport = InputTransport::Direct;
            self.paste = Some(PasteState::new());
            return Ok(());
        }
        let parsed = match crate::vt_input::parse_event(&self.wire_buffer, input_available) {
            Ok(parsed) => parsed,
            Err(_) => {
                // A complete but malformed/unsupported sequence belongs to
                // the terminal protocol. Consume it and recover at the next
                // record instead of returning an error that would terminate
                // the host or replaying the bytes as shell input.
                self.wire_buffer.clear();
                return Ok(());
            }
        };
        match parsed {
            Some(crate::vt_input::Parsed::Event(event)) => {
                self.wire_buffer.clear();
                if self.transport == InputTransport::Unknown {
                    self.transport = InputTransport::Direct;
                }
                self.events.push_back(event);
            }
            Some(crate::vt_input::Parsed::Ignored) => {
                self.wire_buffer.clear();
            }
            Some(crate::vt_input::Parsed::WindowsKey {
                virtual_key,
                scan,
                unicode,
                down,
                control,
                repeat,
            }) => {
                self.wire_buffer.clear();
                self.transport = InputTransport::Win32Envelope;
                let key = KeyRecord {
                    down,
                    repeat,
                    virtual_key,
                    scan,
                    unicode,
                    control_state: control,
                };
                self.consume_decoded_key(key)?;
            }
            None => {}
        }
        Ok(())
    }

    fn feed_payload_parser(&mut self, byte: u8, input_available: bool) -> io::Result<()> {
        if self.payload_discard_until_final {
            if is_vt_final_byte(byte) {
                self.payload_discard_until_final = false;
            }
            return Ok(());
        }
        if self.payload_buffer.len() >= MAX_VT_BUFFER_BYTES {
            self.payload_buffer.clear();
            self.payload_discard_until_final = !is_vt_final_byte(byte);
            return Ok(());
        }
        self.payload_buffer.push(byte);
        let parsed = match crate::vt_input::parse_event(&self.payload_buffer, input_available) {
            Ok(parsed) => parsed,
            Err(_) => {
                // Keep malformed terminal input out of the shell while
                // allowing the next independent key/event to proceed.
                self.payload_buffer.clear();
                return Ok(());
            }
        };
        match parsed {
            Some(crate::vt_input::Parsed::Event(event)) => {
                self.payload_buffer.clear();
                self.events.push_back(event);
            }
            Some(crate::vt_input::Parsed::Ignored) => {
                self.payload_buffer.clear();
            }
            Some(crate::vt_input::Parsed::WindowsKey {
                virtual_key,
                scan,
                unicode,
                down,
                control,
                repeat,
            }) => {
                self.payload_buffer.clear();
                let key = KeyRecord {
                    down,
                    repeat,
                    virtual_key,
                    scan,
                    unicode,
                    control_state: control,
                };
                self.consume_decoded_key(key)?;
            }
            None => {}
        }
        Ok(())
    }

    fn consume_decoded_key(&mut self, key: KeyRecord) -> io::Result<()> {
        // Win32 input mode uses native virtual-key codes for printable ASCII
        // even while a bracketed paste is active. At this layer the marker
        // has already established that the record is paste data, so preserve
        // its Unicode code unit and suppress both press and release events.
        if self.paste.is_some() {
            if is_paste_alt_code_unit(key) {
                return self.consume_paste_code_unit(key);
            }
            if key.down {
                if is_paste_modifier_noise(key) {
                    return Ok(());
                }
                self.consume_paste_code_unit(key)?;
            }
            return Ok(());
        }
        if is_vt_character(&key) {
            if key.down {
                let bytes = utf8_bytes(key.unicode, &mut self.payload_surrogate);
                for _ in 0..key_repeat_count(key) {
                    for byte in bytes.iter().copied() {
                        self.consume_payload_byte(byte, true)?;
                    }
                }
            }
            return Ok(());
        }
        self.push_key_events(key);
        Ok(())
    }

    fn consume_paste_code_unit(&mut self, key: KeyRecord) -> io::Result<()> {
        let bytes = utf8_bytes(key.unicode, &mut self.payload_surrogate);
        for _ in 0..key_repeat_count(key) {
            for byte in bytes.iter().copied() {
                self.consume_payload_byte(byte, true)?;
            }
        }
        Ok(())
    }

    fn push_key_events(&mut self, key: KeyRecord) {
        let repeats = key_repeat_count(key);
        let Some(event) = self.key_event(key) else {
            return;
        };
        self.events.push_back(event.clone());
        for _ in 1..repeats {
            self.events.push_back(event.clone());
        }
    }

    fn key_event(&mut self, key: KeyRecord) -> Option<Event> {
        // crossterm treats the Unicode-bearing VK_MENU release as the final
        // character of an Alt code. The host consumes press events, so expose
        // that character as one press and never leak the modifier release.
        if !key.down && key.virtual_key == VK_MENU && key.unicode != 0 {
            let mut alt_code = key;
            alt_code.down = true;
            alt_code.virtual_key = 0;
            alt_code.scan = 0;
            return self.key_event(alt_code);
        }

        let mut modifiers = key_modifiers(key.control_state);
        // Right-Alt with Ctrl is AltGr on Windows. A printable Unicode value
        // is already the layout-resolved character; retaining Ctrl+Alt would
        // make a printable AltGr character look like a shell shortcut. Keep
        // Ctrl+Alt for zero-Unicode shortcuts such as Ctrl+Alt+Space.
        if key.control_state & RIGHT_ALT_PRESSED != 0
            && modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && is_printable_unicode(key.unicode)
        {
            modifiers.remove(KeyModifiers::CONTROL | KeyModifiers::ALT);
        }

        let only_alt = modifiers == KeyModifiers::ALT;
        if only_alt && (VK_NUMPAD0..=VK_NUMPAD9).contains(&key.virtual_key) {
            // Numpad digits used to compose an Alt code are not individual
            // input characters.
            return None;
        }

        let code = match key.virtual_key {
            VK_SHIFT | VK_CONTROL | VK_MENU => return None,
            VK_BACK => KeyCode::Backspace,
            VK_TAB if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            VK_TAB => KeyCode::Tab,
            VK_RETURN => KeyCode::Enter,
            VK_ESCAPE => KeyCode::Esc,
            VK_PRIOR => KeyCode::PageUp,
            VK_NEXT => KeyCode::PageDown,
            VK_END => KeyCode::End,
            VK_HOME => KeyCode::Home,
            VK_LEFT => KeyCode::Left,
            VK_UP => KeyCode::Up,
            VK_RIGHT => KeyCode::Right,
            VK_DOWN => KeyCode::Down,
            VK_INSERT => KeyCode::Insert,
            VK_DELETE => KeyCode::Delete,
            VK_F1..=VK_F24 => KeyCode::F((key.virtual_key - 0x6f) as u8),
            VK_SPACE if key.unicode == 0 && modifiers.contains(KeyModifiers::CONTROL) => {
                KeyCode::Char(' ')
            }
            _ => self.unicode_key_code(key, modifiers)?,
        };
        // Match crossterm's surrogate lifetime: a valid key event arriving
        // between a high and low surrogate invalidates the pending high
        // surrogate. Key-up records are intentionally excluded because the
        // console repeats Unicode values on release.
        if key.down && !is_utf16_surrogate(key.unicode) {
            self.native_text_surrogate = None;
        }
        let kind = if key.down {
            KeyEventKind::Press
        } else {
            KeyEventKind::Release
        };
        Some(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)))
    }

    fn unicode_key_code(&mut self, key: KeyRecord, modifiers: KeyModifiers) -> Option<KeyCode> {
        let unicode = key.unicode;
        if (0xd800..=0xdfff).contains(&unicode) {
            if !key.down {
                return None;
            }
            if let Some(high) = self.native_text_surrogate.take() {
                return char::decode_utf16([high, unicode])
                    .next()
                    .and_then(Result::ok)
                    .map(KeyCode::Char);
            }
            if (0xd800..=0xdbff).contains(&unicode) {
                self.native_text_surrogate = Some(unicode);
            }
            return None;
        }
        if unicode == 0 {
            if key.control_state & LEFT_ALT_PRESSED != 0
                && modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && (b'A' as u16..=b'Z' as u16).contains(&key.virtual_key)
            {
                return Some(KeyCode::Char(
                    (b'a' + (key.virtual_key as u8 - b'A')) as char,
                ));
            }
            return if key.virtual_key == VK_SPACE && modifiers.contains(KeyModifiers::CONTROL) {
                Some(KeyCode::Char(' '))
            } else {
                None
            };
        }
        if (1..=26).contains(&unicode) && modifiers.contains(KeyModifiers::CONTROL) {
            return Some(KeyCode::Char((b'a' + unicode as u8 - 1) as char));
        }
        let code = match unicode {
            0x09 => KeyCode::Tab,
            0x0d => KeyCode::Enter,
            0x1b => KeyCode::Esc,
            0x7f => KeyCode::Backspace,
            value => KeyCode::Char(char::from_u32(u32::from(value))?),
        };
        Some(code)
    }

    fn mouse_event(&mut self, mouse: MOUSE_EVENT_RECORD) -> Option<Event> {
        let modifiers = key_modifiers(mouse.dwControlKeyState);
        let column = mouse.dwMousePosition.X.max(0) as u16;
        let row = self.relative_mouse_row(mouse.dwMousePosition.Y);
        let state = mouse.dwButtonState;
        let left = state & FROM_LEFT_1ST_BUTTON_PRESSED != 0;
        let right = state
            & (RIGHTMOST_BUTTON_PRESSED
                | FROM_LEFT_3RD_BUTTON_PRESSED
                | FROM_LEFT_4TH_BUTTON_PRESSED)
            != 0;
        let middle = state & FROM_LEFT_2ND_BUTTON_PRESSED != 0;
        let kind = match mouse.dwEventFlags {
            MOUSE_MOVED => {
                if right {
                    MouseEventKind::Drag(MouseButton::Right)
                } else if middle {
                    MouseEventKind::Drag(MouseButton::Middle)
                } else if left {
                    MouseEventKind::Drag(MouseButton::Left)
                } else {
                    MouseEventKind::Moved
                }
            }
            MOUSE_WHEELED => {
                if (state as i32) < 0 {
                    MouseEventKind::ScrollDown
                } else {
                    MouseEventKind::ScrollUp
                }
            }
            MOUSE_HWHEELED => {
                if (state as i32) < 0 {
                    MouseEventKind::ScrollLeft
                } else {
                    MouseEventKind::ScrollRight
                }
            }
            0 | DOUBLE_CLICK => {
                if left && !self.mouse_buttons.left {
                    MouseEventKind::Down(MouseButton::Left)
                } else if !left && self.mouse_buttons.left {
                    MouseEventKind::Up(MouseButton::Left)
                } else if right && !self.mouse_buttons.right {
                    MouseEventKind::Down(MouseButton::Right)
                } else if !right && self.mouse_buttons.right {
                    MouseEventKind::Up(MouseButton::Right)
                } else if middle && !self.mouse_buttons.middle {
                    MouseEventKind::Down(MouseButton::Middle)
                } else if !middle && self.mouse_buttons.middle {
                    MouseEventKind::Up(MouseButton::Middle)
                } else {
                    self.mouse_buttons.left = left;
                    self.mouse_buttons.right = right;
                    self.mouse_buttons.middle = middle;
                    return None;
                }
            }
            _ => return None,
        };
        self.mouse_buttons.left = left;
        self.mouse_buttons.right = right;
        self.mouse_buttons.middle = middle;
        Some(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }))
    }

    fn relative_mouse_row(&self, row: i16) -> u16 {
        let Some(output) = self.output else {
            return row.max(0) as u16;
        };
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
            return row.max(0) as u16;
        }
        row.saturating_sub(info.srWindow.Top).max(0) as u16
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            if !self.input.is_null() && self.input != INVALID_HANDLE_VALUE {
                let _ = CloseHandle(self.input);
            }
            if let Some(output) = self.output
                && !output.is_null()
                && output != INVALID_HANDLE_VALUE
            {
                let _ = CloseHandle(output);
            }
        }
    }
}

fn unmarked_paste_from_input_records(records: &[INPUT_RECORD]) -> Option<String> {
    if records.len() < 2 {
        return None;
    }
    let mut keys = Vec::with_capacity(records.len());
    for record in records {
        if u32::from(record.EventType) != KEY_EVENT {
            return None;
        }
        keys.push(KeyRecord::from(unsafe { record.Event.KeyEvent }));
    }
    if let Some(decoded) = decode_win32_envelope_stream(&keys) {
        unmarked_paste_text(&decoded)
    } else {
        unmarked_paste_text(&keys)
    }
}

/// Decode a complete run of Win32 CSI_ key envelopes without changing the
/// live incremental parser. This is used only to identify an unbracketed text
/// injection batch; bracketed paste contains `CSI 200~` and deliberately
/// fails this decoder so the normal paste transaction consumes it.
fn decode_win32_envelope_stream(wire: &[KeyRecord]) -> Option<Vec<KeyRecord>> {
    let mut bytes = Vec::with_capacity(wire.len());
    for key in wire {
        if !key.down || !is_vt_character(key) || key.unicode > 0x7f {
            return None;
        }
        for _ in 0..key_repeat_count(*key) {
            bytes.push(key.unicode as u8);
        }
    }
    if !bytes.starts_with(b"\x1b[") {
        return None;
    }
    let mut decoded = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        if !bytes[start..].starts_with(b"\x1b[") {
            return None;
        }
        let relative_end = bytes[start + 2..].iter().position(|byte| *byte == b'_')?;
        let end = start + 2 + relative_end;
        let parsed = crate::vt_input::parse_event(&bytes[start..=end], false).ok()??;
        let crate::vt_input::Parsed::WindowsKey {
            virtual_key,
            scan,
            unicode,
            down,
            control,
            repeat,
        } = parsed
        else {
            return None;
        };
        decoded.push(KeyRecord {
            down,
            repeat,
            virtual_key,
            scan,
            unicode,
            control_state: control,
        });
        start = end + 1;
    }
    Some(decoded)
}

fn unmarked_paste_text(keys: &[KeyRecord]) -> Option<String> {
    let modifier_mask = SHIFT_PRESSED
        | LEFT_ALT_PRESSED
        | RIGHT_ALT_PRESSED
        | LEFT_CTRL_PRESSED
        | RIGHT_CTRL_PRESSED;
    let mut units = Vec::with_capacity(keys.len());
    let mut newline = false;
    let mut printable = false;
    for key in keys.iter().copied() {
        if is_paste_modifier_noise(key) {
            continue;
        }
        if !key.down && !is_paste_alt_code_unit(key) {
            return None;
        }
        if key.control_state & modifier_mask != 0 || key.unicode == 0 {
            return None;
        }
        let accepted = matches!(key.unicode, 9 | 10 | 13)
            || is_utf16_surrogate(key.unicode)
            || char::from_u32(u32::from(key.unicode)).is_some_and(|ch| !ch.is_control());
        if !accepted {
            return None;
        }
        newline |= matches!(key.unicode, 10 | 13);
        printable |= !matches!(key.unicode, 9 | 10 | 13) && !is_utf16_surrogate(key.unicode);
        for _ in 0..key_repeat_count(key) {
            units.push(key.unicode);
            if units.len() > MAX_PASTE_BYTES {
                return None;
            }
        }
    }
    if !newline || !printable {
        return None;
    }
    String::from_utf16(&units).ok()
}

/// A terminal which ignores bracketed-paste mode can still enqueue a whole
/// clipboard operation as one console batch. At the event layer physical
/// presses and releases are already decoded. Coalesce only a contiguous text
/// run which has printable input on both sides of a newline; an ordinary
/// command followed by Enter therefore keeps its execution semantics.
fn coalesce_unmarked_multiline_events(events: Vec<Event>) -> Vec<Event> {
    let mut output = Vec::with_capacity(events.len());
    let mut index = 0usize;
    while index < events.len() {
        let start = index;
        let mut text = String::new();
        let mut first_newline = None;
        let mut text_units = 0usize;
        let mut last_was_newline = false;
        while index < events.len() {
            let Event::Key(key) = &events[index] else {
                break;
            };
            if key.kind == KeyEventKind::Release {
                index += 1;
                continue;
            }
            match key.code {
                KeyCode::Char(character)
                    if !character.is_control()
                        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
                {
                    text.push(character);
                    text_units += 1;
                    last_was_newline = false;
                }
                KeyCode::Tab if key.modifiers.is_empty() => {
                    text.push('\t');
                    text_units += 1;
                    last_was_newline = false;
                }
                KeyCode::Enter if key.modifiers.is_empty() => {
                    if !last_was_newline {
                        first_newline.get_or_insert(text_units);
                        text.push('\n');
                    }
                    last_was_newline = true;
                }
                // ConPTY represents the LF half of a pasted CRLF pair as a
                // Ctrl+Enter record. It immediately follows the ordinary
                // Enter generated for CR and carries no additional newline.
                KeyCode::Enter if key.modifiers == KeyModifiers::CONTROL && last_was_newline => {}
                _ => break,
            }
            index += 1;
        }
        let qualifies = first_newline.is_some_and(|before| before > 0 && text_units > before)
            && text.len() <= MAX_PASTE_BYTES;
        if qualifies {
            output.push(Event::Paste(text));
        } else if index > start {
            output.extend(events[start..index].iter().cloned());
        } else {
            output.push(events[index].clone());
            index += 1;
        }
    }
    output
}

fn trailing_text_run_start(events: &[Event]) -> usize {
    let mut start = 0;
    for (index, event) in events.iter().enumerate() {
        let Event::Key(key) = event else {
            start = index + 1;
            continue;
        };
        if key.kind != KeyEventKind::Release && !is_unmarked_text_key(key) {
            start = index + 1;
        }
    }
    start
}

fn text_event_run(events: &[Event], start: usize) -> (usize, String) {
    let mut index = start;
    let mut text = String::new();
    let mut last_was_newline = false;
    while index < events.len() {
        let Event::Key(key) = &events[index] else {
            break;
        };
        if key.kind == KeyEventKind::Release {
            index += 1;
            continue;
        }
        match key.code {
            KeyCode::Char(character)
                if !character.is_control()
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
            {
                text.push(character);
                last_was_newline = false;
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                text.push('\t');
                last_was_newline = false;
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                if !last_was_newline {
                    text.push('\n');
                }
                last_was_newline = true;
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::CONTROL && last_was_newline => {}
            _ => break,
        }
        index += 1;
    }
    (index, text)
}

fn is_unmarked_text_key(key: &KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Char(character)
            if !character.is_control()
                && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
    ) || matches!(key.code, KeyCode::Tab if key.modifiers.is_empty())
        || matches!(key.code, KeyCode::Enter if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL)
}

fn clipboard_multiline_text() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("BLUEBERRY_TEST_CLIPBOARD_TEXT") {
        let value = normalize_clipboard_newlines(value);
        return value.contains('\n').then_some(value);
    }

    // Clipboard access is deliberately best-effort. A busy clipboard keeps
    // the ordinary key path unchanged; its contents are never logged.
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(u32::from(CF_UNICODETEXT));
        if handle.is_null() {
            CloseClipboard();
            return None;
        }
        let pointer = GlobalLock(handle) as *const u16;
        if pointer.is_null() {
            CloseClipboard();
            return None;
        }
        let max_units = MAX_PASTE_BYTES / 2;
        let mut length = 0;
        while length < max_units && *pointer.add(length) != 0 {
            length += 1;
        }
        let value = (length < max_units)
            .then(|| String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length)));
        GlobalUnlock(handle);
        CloseClipboard();
        let value = normalize_clipboard_newlines(value?);
        value.contains('\n').then_some(value)
    }
}

fn normalize_clipboard_newlines(value: String) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_vt_character(key: &KeyRecord) -> bool {
    key.virtual_key == 0 && key.scan == 0
}

fn is_vt_final_byte(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

fn is_utf16_surrogate(unit: u16) -> bool {
    (0xd800..=0xdfff).contains(&unit)
}

fn is_paste_modifier_noise(key: KeyRecord) -> bool {
    if key.unicode != 0 {
        return false;
    }
    if matches!(key.virtual_key, VK_SHIFT | VK_CONTROL | VK_MENU) {
        return true;
    }
    // Windows represents an Alt-code's numeric composition as Alt-held
    // numpad records with no Unicode value. They are not paste characters;
    // the completed character arrives on the VK_MENU release below.
    let state = key.control_state;
    let only_alt = state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0
        && state & (SHIFT_PRESSED | LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) == 0;
    only_alt && (VK_NUMPAD0..=VK_NUMPAD9).contains(&key.virtual_key)
}

fn is_paste_alt_code_unit(key: KeyRecord) -> bool {
    key.virtual_key == VK_MENU && key.unicode != 0
}

fn is_printable_unicode(unit: u16) -> bool {
    if is_utf16_surrogate(unit) {
        return true;
    }
    char::from_u32(u32::from(unit)).is_some_and(|ch| !ch.is_control())
}

fn is_physical_escape_release(key: KeyRecord) -> bool {
    !key.down && key.virtual_key == VK_ESCAPE
}

fn key_repeat_count(key: KeyRecord) -> usize {
    if key.down {
        usize::from(key.repeat.max(1))
    } else {
        1
    }
}

fn key_modifiers(state: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();
    if state & SHIFT_PRESSED != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    modifiers
}

fn utf8_bytes(unit: u16, surrogate: &mut Option<u16>) -> Vec<u8> {
    if (0xd800..=0xdfff).contains(&unit) {
        if let Some(high) = surrogate.take() {
            let ch = char::decode_utf16([high, unit])
                .next()
                .and_then(Result::ok)
                .unwrap_or('\u{fffd}');
            return ch.to_string().into_bytes();
        }
        if (0xd800..=0xdbff).contains(&unit) {
            *surrogate = Some(unit);
        }
        return Vec::new();
    }
    *surrogate = None;
    char::from_u32(u32::from(unit))
        .unwrap_or('\u{fffd}')
        .to_string()
        .into_bytes()
}

fn open_console_device(name: &str) -> io::Result<HANDLE> {
    let wide: Vec<u16> = format!("{name}\0").encode_utf16().collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_reader() -> Reader {
        Reader {
            input: null_mut(),
            output: None,
            records: VecDeque::new(),
            events: VecDeque::new(),
            wire_buffer: Vec::new(),
            payload_buffer: Vec::new(),
            payload_prefix: Vec::new(),
            transport: InputTransport::Unknown,
            paste: None,
            wire_discard_until_final: false,
            payload_discard_until_final: false,
            wire_surrogate: None,
            payload_surrogate: None,
            native_text_surrogate: None,
            mouse_buttons: MouseButtons::default(),
            paste_rejection: None,
            unmarked_paste: None,
        }
    }

    fn vt_key(unicode: u16) -> KeyRecord {
        KeyRecord {
            down: true,
            repeat: 1,
            virtual_key: 0,
            scan: 0,
            unicode,
            control_state: 0,
        }
    }

    fn native_key(
        down: bool,
        virtual_key: u16,
        scan: u16,
        unicode: u16,
        control_state: u32,
    ) -> KeyRecord {
        KeyRecord {
            down,
            repeat: 1,
            virtual_key,
            scan,
            unicode,
            control_state,
        }
    }

    fn feed_bytes(reader: &mut Reader, bytes: &[u8]) {
        for &byte in bytes {
            reader.consume_key(vt_key(u16::from(byte))).unwrap();
        }
    }

    fn feed_envelope(reader: &mut Reader, key: KeyRecord) {
        let bytes = format!(
            "\x1b[{};{};{};{};{};{}_",
            key.virtual_key,
            key.scan,
            key.unicode,
            u8::from(key.down),
            key.control_state,
            key.repeat,
        );
        feed_bytes(reader, bytes.as_bytes());
    }

    fn drain(reader: &mut Reader) -> Vec<Event> {
        reader.events.drain(..).collect()
    }

    fn key_code(event: &Event) -> Option<KeyCode> {
        match event {
            Event::Key(key) => Some(key.code),
            _ => None,
        }
    }

    #[test]
    fn direct_records_decode_sequences_and_adjacent_escape() {
        let mut reader = test_reader();
        feed_bytes(&mut reader, b"\x1b[A\x1b[24~");
        assert_eq!(
            drain(&mut reader),
            vec![
                Event::Key(KeyCode::Up.into()),
                Event::Key(KeyCode::F(12).into()),
            ]
        );

        reader.consume_key(vt_key(PASTE_START[0] as u16)).unwrap();
        reader
            .consume_key(native_key(false, VK_ESCAPE, 1, 0, 0))
            .unwrap();
        assert_eq!(drain(&mut reader), vec![Event::Key(KeyCode::Esc.into())]);

        // Two ESC records are two physical keys. The second one is the
        // beginning of the following paste marker.
        feed_bytes(&mut reader, b"\x1b\x1b");
        assert_eq!(drain(&mut reader), vec![Event::Key(KeyCode::Esc.into())]);
        feed_bytes(&mut reader, &PASTE_START[1..]);
        feed_bytes(&mut reader, b"one\x1b[201~Z");
        let events = drain(&mut reader);
        assert_eq!(events[0], Event::Paste("one".into()));
        assert_eq!(key_code(&events[1]), Some(KeyCode::Char('Z')));
        if let Event::Key(key) = &events[1] {
            assert!(!key.modifiers.contains(KeyModifiers::ALT));
        }

        reader
            .consume_key(KeyRecord {
                down: true,
                repeat: 3,
                virtual_key: 0,
                scan: 0,
                unicode: b'b' as u16,
                control_state: 0,
            })
            .unwrap();
        assert_eq!(
            drain(&mut reader),
            vec![
                Event::Key(KeyCode::Char('b').into()),
                Event::Key(KeyCode::Char('b').into()),
                Event::Key(KeyCode::Char('b').into()),
            ]
        );
    }

    #[test]
    fn win32_envelopes_decode_nested_vt_and_surrogates() {
        let mut reader = test_reader();
        for byte in b"\x1b[A" {
            feed_envelope(&mut reader, native_key(true, 0, 0, u16::from(*byte), 0));
        }
        assert_eq!(drain(&mut reader), vec![Event::Key(KeyCode::Up.into())]);

        // The outer CSI_ envelopes are ASCII. An inner high surrogate must
        // survive those outer records until its low surrogate arrives.
        feed_envelope(&mut reader, native_key(true, 0, 0, 0xd83d, 0));
        feed_envelope(&mut reader, native_key(true, 0, 0, 0xde00, 0));
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('😀').into())]
        );
    }

    #[test]
    fn paste_handles_native_ascii_crlf_and_alt_code_emoji_records() {
        let mut reader = test_reader();
        for &byte in PASTE_START {
            feed_envelope(&mut reader, native_key(true, 0, 0, u16::from(byte), 0));
        }
        for ch in "你".encode_utf16() {
            feed_envelope(&mut reader, native_key(true, 0, 0, ch, 0));
        }
        // This is the shape emitted by ConPTY for an emoji while Win32 input
        // mode is active: Alt+numpad key-down noise, followed by the high and
        // low UTF-16 units on VK_MENU releases.
        let alt = LEFT_ALT_PRESSED;
        feed_envelope(&mut reader, native_key(true, VK_MENU, 56, 0, alt));
        feed_envelope(&mut reader, native_key(true, VK_NUMPAD0 + 6, 77, 0, alt));
        feed_envelope(&mut reader, native_key(false, VK_NUMPAD0 + 6, 77, 0, alt));
        feed_envelope(&mut reader, native_key(false, VK_MENU, 56, 0xd83d, 0));
        feed_envelope(&mut reader, native_key(true, VK_MENU, 56, 0, alt));
        feed_envelope(&mut reader, native_key(true, VK_NUMPAD0 + 2, 79, 0, alt));
        feed_envelope(&mut reader, native_key(false, VK_NUMPAD0 + 2, 79, 0, alt));
        feed_envelope(&mut reader, native_key(false, VK_MENU, 56, 0xde00, 0));
        for ch in "好".encode_utf16() {
            feed_envelope(&mut reader, native_key(true, 0, 0, ch, 0));
        }
        for &byte in b"\r\n" {
            feed_envelope(
                &mut reader,
                native_key(true, VK_RETURN, 28, u16::from(byte), 0),
            );
        }
        for &byte in PASTE_END {
            feed_envelope(&mut reader, native_key(true, 0, 0, u16::from(byte), 0));
        }
        assert_eq!(drain(&mut reader), vec![Event::Paste("你😀好\r\n".into())]);
    }

    #[test]
    fn unmarked_multiline_injection_is_coalesced_without_timing() {
        let text = "Get-PnpDevice -PresentOnly |\r\nWhere-Object {$_.InstanceId -like 'PCI\\VEN_15B7*'} |\r\nFormat-List *";
        let keys = text
            .encode_utf16()
            .map(|unit| native_key(true, 0, 0, unit, 0))
            .collect::<Vec<_>>();
        assert_eq!(unmarked_paste_text(&keys).as_deref(), Some(text));

        let mut physical = Vec::new();
        for unit in "one\r\ntwo".encode_utf16() {
            physical.push(native_key(true, unit, 1, unit, 0));
            physical.push(native_key(false, unit, 1, unit, 0));
        }
        assert!(unmarked_paste_text(&physical).is_none());
    }

    #[test]
    fn unmarked_win32_envelope_batch_decodes_before_coalescing() {
        let text = "第一行\r\n第二行😀";
        let logical = text
            .encode_utf16()
            .map(|unit| native_key(true, 0, 0, unit, 0))
            .collect::<Vec<_>>();
        let mut wire = Vec::new();
        for key in logical {
            let envelope = format!(
                "\x1b[{};{};{};{};{};{}_",
                key.virtual_key,
                key.scan,
                key.unicode,
                u8::from(key.down),
                key.control_state,
                key.repeat,
            );
            wire.extend(
                envelope
                    .bytes()
                    .map(|byte| native_key(true, 0, 0, u16::from(byte), 0)),
            );
        }
        let decoded = decode_win32_envelope_stream(&wire).unwrap();
        assert_eq!(unmarked_paste_text(&decoded).as_deref(), Some(text));
    }

    #[test]
    fn decoded_multiline_batch_coalesces_but_trailing_enter_executes() {
        fn pair(code: KeyCode) -> [Event; 2] {
            [
                Event::Key(KeyEvent::new_with_kind(
                    code,
                    KeyModifiers::empty(),
                    KeyEventKind::Press,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    code,
                    KeyModifiers::empty(),
                    KeyEventKind::Release,
                )),
            ]
        }
        let mut multiline = Vec::new();
        for code in [
            KeyCode::Char('一'),
            KeyCode::Enter,
            KeyCode::Enter,
            KeyCode::Char('二'),
        ] {
            multiline.extend(pair(code));
        }
        assert_eq!(
            coalesce_unmarked_multiline_events(multiline),
            vec![Event::Paste("一\n二".into())]
        );

        let mut command = Vec::new();
        for code in [KeyCode::Char('g'), KeyCode::Char('i'), KeyCode::Enter] {
            command.extend(pair(code));
        }
        let result = coalesce_unmarked_multiline_events(command);
        assert!(!result.iter().any(|event| matches!(event, Event::Paste(_))));
    }

    #[test]
    fn unmarked_clipboard_transaction_survives_reader_batches() {
        let mut reader = test_reader();
        reader.unmarked_paste = Some(PendingUnmarkedPaste {
            target: "one\ntwo".into(),
            events: vec![
                Event::Key(KeyCode::Char('o').into()),
                Event::Key(KeyCode::Char('n').into()),
                Event::Key(KeyCode::Char('e').into()),
                Event::Key(KeyCode::Enter.into()),
            ],
        });
        let ready = reader.finish_unmarked_paste(vec![
            Event::Key(KeyCode::Char('t').into()),
            Event::Key(KeyCode::Char('w').into()),
            Event::Key(KeyCode::Char('o').into()),
        ]);
        assert_eq!(ready, vec![Event::Paste("one\ntwo".into())]);
        assert!(reader.unmarked_paste.is_none());
    }

    #[test]
    fn native_key_semantics_preserve_shortcuts_and_altgr() {
        let mut reader = test_reader();
        reader
            .consume_key(native_key(
                true,
                VK_SPACE,
                57,
                0,
                LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED,
            ))
            .unwrap();
        let events = drain(&mut reader);
        assert_eq!(events.len(), 1);
        if let Event::Key(key) = &events[0] {
            assert_eq!(key.code, KeyCode::Char(' '));
            assert!(key.modifiers.contains(KeyModifiers::CONTROL));
            assert!(key.modifiers.contains(KeyModifiers::ALT));
        } else {
            panic!("expected Ctrl+Alt+Space");
        }

        reader
            .consume_key(native_key(
                true,
                0x51,
                16,
                b'@' as u16,
                LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED,
            ))
            .unwrap();
        let events = drain(&mut reader);
        assert_eq!(events.len(), 1);
        if let Event::Key(key) = &events[0] {
            assert_eq!(key.code, KeyCode::Char('@'));
            assert!(key.modifiers.is_empty());
        } else {
            panic!("expected AltGr character");
        }

        reader
            .consume_key(native_key(
                true,
                VK_NUMPAD0 + 2,
                79,
                b'2' as u16,
                RIGHT_ALT_PRESSED,
            ))
            .unwrap();
        assert!(drain(&mut reader).is_empty());

        reader
            .consume_key(native_key(
                false,
                VK_MENU,
                56,
                b'1' as u16,
                RIGHT_ALT_PRESSED,
            ))
            .unwrap();
        let events = drain(&mut reader);
        assert_eq!(events.len(), 1);
        if let Event::Key(key) = &events[0] {
            assert_eq!(key.code, KeyCode::Char('1'));
            assert_eq!(key.kind, KeyEventKind::Press);
        } else {
            panic!("expected Alt-code character");
        }

        // A key-up record between UTF-16 halves must not discard the pending
        // high surrogate.
        reader
            .consume_key(native_key(true, 0x41, 30, 0xd83d, 0))
            .unwrap();
        reader
            .consume_key(native_key(false, 0x41, 30, 0xd83d, 0))
            .unwrap();
        reader
            .consume_key(native_key(true, 0x41, 30, 0xde00, 0))
            .unwrap();
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('😀').into())]
        );
    }

    #[test]
    fn malformed_and_oversized_sequences_recover_before_next_key() {
        let mut reader = test_reader();
        feed_bytes(&mut reader, b"a");
        drain(&mut reader);

        // parse_event reports this complete malformed cursor response as an
        // error. Reader consumes it and remains usable.
        feed_bytes(&mut reader, b"\x1b[0;0Rz");
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('z').into())]
        );

        // An incomplete control prefix is bounded and dropped through its
        // final byte, so it cannot grow while waiting for a hostile stream.
        feed_bytes(&mut reader, b"\x1b[");
        for _ in 0..=MAX_VT_BUFFER_BYTES {
            reader.consume_key(vt_key(b'0' as u16)).unwrap();
        }
        feed_bytes(&mut reader, b"xq");
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('q').into())]
        );

        // If the byte that reaches the bound is already a sequence final,
        // finish discarding immediately and preserve the next independent
        // key. Exercise the same path for the outer wire buffer as well.
        feed_bytes(&mut reader, b"\x1b[");
        for _ in 0..(MAX_VT_BUFFER_BYTES - 3) {
            reader.consume_key(vt_key(b'0' as u16)).unwrap();
        }
        feed_bytes(&mut reader, b"xq");
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('q').into())]
        );

        reader.transport = InputTransport::Win32Envelope;
        feed_bytes(&mut reader, b"\x1b[");
        for _ in 0..(MAX_VT_BUFFER_BYTES - 3) {
            reader.consume_key(vt_key(b'0' as u16)).unwrap();
        }
        feed_bytes(&mut reader, b"xq");
        assert_eq!(
            drain(&mut reader),
            vec![Event::Key(KeyCode::Char('q').into())]
        );

        reader.transport = InputTransport::Direct;
        feed_bytes(&mut reader, PASTE_START);
        let mut remaining = MAX_PASTE_BYTES + 1;
        while remaining != 0 {
            let repeat = remaining.min(usize::from(u16::MAX)) as u16;
            reader
                .consume_key(KeyRecord {
                    down: true,
                    repeat,
                    virtual_key: 0,
                    scan: 0,
                    unicode: b'x' as u16,
                    control_state: 0,
                })
                .unwrap();
            remaining -= usize::from(repeat);
        }
        feed_bytes(&mut reader, PASTE_END);
        assert_eq!(
            reader.take_paste_rejection(),
            Some(PasteRejection::TooLarge)
        );
        feed_bytes(&mut reader, b"q");
        let events = drain(&mut reader);
        assert_eq!(events, vec![Event::Key(KeyCode::Char('q').into())]);
    }
}
