//! Streaming private OSC transport. Unknown terminal controls remain byte-for-byte intact.
use serde_json::Value;

#[derive(Debug)]
pub enum Part {
    Data(Vec<u8>),
    Message(Value),
    CursorQuery(bool),
}

pub struct Decoder {
    token: String,
    private_prefix: Vec<u8>,
    pending: Vec<u8>,
    state: u8,
}

impl Decoder {
    pub fn new(token: String) -> Self {
        Self {
            private_prefix: format!("\x1b]7776;{token};").into_bytes(),
            token,
            pending: Vec::new(),
            state: 0,
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<Part> {
        let mut parts = Vec::new();
        let mut plain = Vec::new();
        for &b in data {
            match self.state {
                0 if b == 0x1b => {
                    self.pending.push(b);
                    self.state = 1;
                }
                0 => plain.push(b),
                1 if b == b']' => {
                    self.pending.push(b);
                    self.state = 2;
                }
                1 if b == b'[' => {
                    self.pending.push(b);
                    self.state = 3;
                }
                1 => {
                    plain.append(&mut self.pending);
                    if b == 0x1b {
                        self.pending.push(b);
                    } else {
                        plain.push(b);
                        self.state = 0;
                    }
                }
                3 => {
                    self.pending.push(b);
                    if (0x40..=0x7e).contains(&b) || self.pending.len() > 4096 {
                        if self.pending == b"\x1b[6n" || self.pending == b"\x1b[?6n" {
                            if !plain.is_empty() {
                                parts.push(Part::Data(std::mem::take(&mut plain)));
                            }
                            parts.push(Part::CursorQuery(self.pending == b"\x1b[?6n"));
                            self.pending.clear();
                        } else {
                            plain.append(&mut self.pending);
                        }
                        self.state = 0;
                    }
                }
                _ => {
                    self.pending.push(b);
                    let bel = b == 7;
                    let st = self.pending.ends_with(b"\x1b\\");
                    if bel || st {
                        let end = self.pending.len() - if bel { 1 } else { 2 };
                        let prefix = format!("7776;{};", self.token);
                        let content = &self.pending[2..end];
                        if let Some(json) = content.strip_prefix(prefix.as_bytes()) {
                            if !plain.is_empty() {
                                parts.push(Part::Data(std::mem::take(&mut plain)));
                            }
                            if let Ok(message) = serde_json::from_slice(json) {
                                parts.push(Part::Message(message));
                            }
                        } else {
                            plain.append(&mut self.pending);
                        }
                        self.pending.clear();
                        self.state = 0;
                    } else if self.pending.len() > 1_048_576
                        && (!self.pending.starts_with(&self.private_prefix)
                            || self.pending.len() > 32 * 1_048_576)
                    {
                        // A 1 MiB UTF-8 paste can expand several times under
                        // PowerShell's ASCII JSON encoder, and buffer.context
                        // may repeat the current token. Pipe frames exceeding
                        // 1 MiB fall back to this authenticated OSC path.
                        // Unknown OSC retains the smaller defensive bound.
                        plain.append(&mut self.pending);
                        self.state = 0;
                    }
                }
            }
        }
        if !plain.is_empty() {
            parts.push(Part::Data(plain));
        }
        parts
    }

    pub fn finish(&mut self) -> Vec<u8> {
        self.state = 0;
        std::mem::take(&mut self.pending)
    }
}

pub fn utf16_to_byte(value: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in value.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(value.len())
}

pub fn byte_to_utf16(value: &str, offset: usize) -> Option<usize> {
    value.get(..offset).map(|s| s.encode_utf16().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn osc_survives_every_chunk_boundary_and_semicolon_in_json() {
        let input = b"before\x1b]7776;secret;{\"event\":\"buffer\",\"line\":\"a;b\"}\x07after";
        for split in 0..=input.len() {
            let mut decoder = Decoder::new("secret".into());
            let mut parts = decoder.feed(&input[..split]);
            parts.extend(decoder.feed(&input[split..]));
            let mut output = Vec::new();
            let mut messages = Vec::new();
            for part in parts {
                match part {
                    Part::Data(d) => output.extend(d),
                    Part::Message(v) => messages.push(v),
                    Part::CursorQuery(_) => panic!(),
                }
            }
            assert_eq!(output, b"beforeafter");
            assert_eq!(messages[0]["line"], "a;b");
        }
    }
    #[test]
    fn ascii_escaped_unicode_paste_survives_the_pipe_fallback_frame_size() {
        let mut frame = b"\x1b]7776;secret;{\"event\":\"buffer\",\"line\":\"".to_vec();
        for _ in 0..200_000 {
            frame.extend_from_slice(b"\\u4f60");
        }
        frame.extend_from_slice(b"\",\"cursor\":200000}\x07");
        let mut decoder = Decoder::new("secret".into());
        let mut messages = Vec::new();
        for chunk in frame.chunks(16_384) {
            for part in decoder.feed(chunk) {
                match part {
                    Part::Message(value) => messages.push(value),
                    _ => panic!("private Unicode frame escaped into terminal output"),
                }
            }
        }
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["line"].as_str().unwrap(), "你".repeat(200_000));
    }
    #[test]
    fn unrelated_osc_is_unchanged() {
        let raw = b"\x1b]52;c;aGVsbG8=\x1b\\\x1b]7776;wrong;{}\x07";
        let mut decoder = Decoder::new("secret".into());
        let out: Vec<u8> = decoder
            .feed(raw)
            .into_iter()
            .flat_map(|p| match p {
                Part::Data(d) => d,
                _ => panic!(),
            })
            .collect();
        assert_eq!(out, raw);
    }
    #[test]
    fn offset_units_are_not_interchangeable() {
        assert_eq!(utf16_to_byte("a你😀b", 4), Some(8));
        assert_eq!(utf16_to_byte("a你😀b", 3), None);
        assert_eq!(byte_to_utf16("a你😀b", 8), Some(4));
        assert_eq!(byte_to_utf16("a你😀b", 2), None);
    }
    #[test]
    fn cursor_queries_are_handled_at_every_split_without_swallowing_styling() {
        let bytes = b"\x1b[32mtext\x1b[6n\x1b[?6n\x1b[0m";
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::new("token".into());
            let mut parts = decoder.feed(&bytes[..split]);
            parts.extend(decoder.feed(&bytes[split..]));
            let mut output = Vec::new();
            let mut queries = Vec::new();
            for part in parts {
                match part {
                    Part::Data(d) => output.extend(d),
                    Part::CursorQuery(p) => queries.push(p),
                    _ => panic!(),
                }
            }
            assert_eq!(output, b"\x1b[32mtext\x1b[0m");
            assert_eq!(queries, [false, true]);
        }
    }
}
