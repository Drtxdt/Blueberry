use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers as Mod};

#[derive(Debug)]
pub enum Input {
    Bytes(Vec<u8>),
    Tab,
    BackTab,
    Previous,
    Next,
    Dismiss,
    Trigger,
    Refresh,
    Reload,
    Resize(u16, u16),
}

pub fn translate(event: Event, application_cursor: bool) -> Option<Input> {
    match event {
        Event::Resize(w, h) => Some(Input::Resize(w, h)),
        Event::Paste(s) => Some(Input::Bytes(s.into_bytes())),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let ctrl = key.modifiers.contains(Mod::CONTROL);
            let alt = key.modifiers.contains(Mod::ALT);
            let shift = key.modifiers.contains(Mod::SHIFT);
            if ctrl && key.code == KeyCode::Char(' ') {
                return Some(Input::Trigger);
            }
            if ctrl && alt && key.code == KeyCode::Char('r') {
                return Some(Input::Reload);
            }
            if ctrl && alt && key.code == KeyCode::Char('c') {
                return Some(Input::Refresh);
            }
            if key.code == KeyCode::Tab && !alt && !ctrl {
                return Some(Input::Tab);
            }
            if key.code == KeyCode::BackTab {
                return Some(Input::BackTab);
            }
            if key.code == KeyCode::Down && !ctrl && !alt && !shift {
                return Some(Input::Next);
            }
            if key.code == KeyCode::Up && !ctrl && !alt && !shift {
                return Some(Input::Previous);
            }
            if key.code == KeyCode::Esc {
                return Some(Input::Dismiss);
            }
            #[cfg(windows)]
            if ctrl && key.code == KeyCode::Backspace {
                // Preserve the Windows Ctrl+Backspace binding, which is not
                // equivalent to Ctrl+W in PSReadLine's Windows key map.
                return Some(Input::Bytes(
                    b"\x1b[8;14;127;1;8;1_\x1b[8;14;127;0;8;1_".to_vec(),
                ));
            }
            let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
            let csi_key = |final_char| {
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{final_char}")
                } else {
                    format!(
                        "\x1b{}{final_char}",
                        if application_cursor { "O" } else { "[" }
                    )
                }
            };
            let tilde_key = |n| {
                if modifier > 1 {
                    format!("\x1b[{n};{modifier}~")
                } else {
                    format!("\x1b[{n}~")
                }
            };
            let sequence = match key.code {
                KeyCode::Char(c) if ctrl && c.is_ascii() => {
                    let b = c.to_ascii_uppercase() as u8;
                    if (b'@'..=b'_').contains(&b) {
                        String::from((b & 0x1f) as char)
                    } else {
                        c.to_string()
                    }
                }
                KeyCode::Char(c) => c.to_string(),
                KeyCode::Enter => "\r".into(),
                KeyCode::Backspace => if ctrl { "\x17" } else { "\x7f" }.into(),
                KeyCode::Left => csi_key('D'),
                KeyCode::Right => csi_key('C'),
                KeyCode::Up => csi_key('A'),
                KeyCode::Down => csi_key('B'),
                KeyCode::Home => csi_key('H'),
                KeyCode::End => csi_key('F'),
                KeyCode::Delete => tilde_key(3),
                KeyCode::Insert => tilde_key(2),
                KeyCode::PageUp => tilde_key(5),
                KeyCode::PageDown => tilde_key(6),
                KeyCode::Tab => "\t".into(),
                KeyCode::F(n) => match n {
                    1..=4 if modifier == 1 => format!("\x1bO{}", (b'P' + n - 1) as char),
                    1..=4 => csi_key((b'P' + n - 1) as char),
                    5..=12 => tilde_key([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
                    _ => return None,
                },
                _ => return None,
            };
            let mut bytes = Vec::new();
            if alt
                && matches!(
                    key.code,
                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace
                )
            {
                bytes.push(0x1b);
            }
            bytes.extend_from_slice(sequence.as_bytes());
            Some(Input::Bytes(bytes))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    #[test]
    fn unicode_and_ctrl_c_are_forwarded() {
        let check = |code, mods, expected: &[u8]| match translate(
            Event::Key(KeyEvent::new(code, mods)),
            false,
        )
        .unwrap()
        {
            Input::Bytes(b) => assert_eq!(b, expected),
            _ => panic!(),
        };
        check(KeyCode::Char('你'), Mod::NONE, "你".as_bytes());
        check(KeyCode::Char('c'), Mod::CONTROL, b"\x03");
        check(KeyCode::Left, Mod::CONTROL, b"\x1b[1;5D");
    }
}
