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
    Native,
    Details,
    Refresh,
    Reload,
    Resize(u16, u16),
}

pub fn parse_chord(text: &str) -> Result<(KeyCode, Mod), String> {
    let mut modifiers = Mod::NONE;
    let mut key = None;
    for part in text.split('+') {
        let part = part.trim().to_ascii_lowercase();
        match part.as_str() {
            "ctrl" | "control" => modifiers |= Mod::CONTROL,
            "alt" => modifiers |= Mod::ALT,
            "shift" => modifiers |= Mod::SHIFT,
            _ => {
                if key.is_some() {
                    return Err("use one key with optional Ctrl/Alt/Shift modifiers".into());
                }
                key = Some(match part.as_str() {
                    "space" => KeyCode::Char(' '),
                    "tab" => KeyCode::Tab,
                    "enter" => KeyCode::Enter,
                    "esc" | "escape" => KeyCode::Esc,
                    _ if part.starts_with('f') && part.len() > 1 => {
                        let n = part[1..]
                            .parse::<u8>()
                            .map_err(|_| "invalid function key")?;
                        if !(1..=12).contains(&n) {
                            return Err("function keys must be F1 through F12".into());
                        }
                        KeyCode::F(n)
                    }
                    _ if part.chars().count() == 1 => KeyCode::Char(part.chars().next().unwrap()),
                    _ => return Err(format!("unsupported key '{part}'")),
                });
            }
        }
    }
    key.map(|key| (key, modifiers))
        .ok_or_else(|| "missing key".into())
}

pub fn configured(event: &Event, keys: &crate::config::KeyBindings) -> Option<Input> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let code = match key.code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };
    for (chord, action) in [
        (&keys.native, Input::Native),
        (&keys.trigger, Input::Trigger),
        (&keys.details, Input::Details),
        (&keys.refresh, Input::Refresh),
        (&keys.reload, Input::Reload),
    ] {
        if parse_chord(chord).ok() == Some((code, key.modifiers)) {
            return Some(action);
        }
    }
    None
}

pub fn protocol_chord(prefix: &str, suffix: char) -> Vec<u8> {
    let number = match prefix {
        "F5" => 15,
        "F6" => 17,
        "F7" => 18,
        "F8" => 19,
        "F9" => 20,
        "F10" => 21,
        "F11" => 23,
        _ => 24,
    };
    format!("\x1b[{number}~{suffix}").into_bytes()
}

pub fn mouse_bytes(event: crossterm::event::MouseEvent, screen: &vt100::Screen) -> Option<Vec<u8>> {
    use crossterm::event::{MouseButton, MouseEventKind as Kind};
    use vt100::{MouseProtocolEncoding as Encoding, MouseProtocolMode as Mode};
    let mode = screen.mouse_protocol_mode();
    if mode == Mode::None {
        return None;
    }
    let button = |button| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let release = matches!(event.kind, Kind::Up(_));
    let mut code: u16 = match event.kind {
        Kind::Down(b) => button(b),
        Kind::Up(b) if mode != Mode::Press => button(b),
        Kind::Drag(b) if matches!(mode, Mode::ButtonMotion | Mode::AnyMotion) => button(b) + 32,
        Kind::Moved if mode == Mode::AnyMotion => 35,
        Kind::ScrollUp => 64,
        Kind::ScrollDown => 65,
        Kind::ScrollLeft => 66,
        Kind::ScrollRight => 67,
        _ => return None,
    };
    if event.modifiers.contains(Mod::SHIFT) {
        code |= 4;
    }
    if event.modifiers.contains(Mod::ALT) {
        code |= 8;
    }
    if event.modifiers.contains(Mod::CONTROL) {
        code |= 16;
    }
    let (x, y) = (u32::from(event.column) + 1, u32::from(event.row) + 1);
    match screen.mouse_protocol_encoding() {
        Encoding::Sgr => {
            Some(format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes())
        }
        encoding => {
            if release {
                code = (code & !3) | 3;
            }
            let mut bytes = b"\x1b[M".to_vec();
            for value in [u32::from(code) + 32, x + 32, y + 32] {
                if encoding == Encoding::Utf8 {
                    let mut buf = [0; 4];
                    bytes
                        .extend_from_slice(char::from_u32(value)?.encode_utf8(&mut buf).as_bytes());
                } else {
                    bytes.push(u8::try_from(value).ok()?);
                }
            }
            Some(bytes)
        }
    }
}

pub fn translate(event: Event, application_cursor: bool) -> Option<Input> {
    match event {
        Event::Resize(w, h) => Some(Input::Resize(w, h)),
        Event::Paste(s) => Some(Input::Bytes(s.into_bytes())),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let ctrl = key.modifiers.contains(Mod::CONTROL);
            let alt = key.modifiers.contains(Mod::ALT);
            let shift = key.modifiers.contains(Mod::SHIFT);
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
                KeyCode::Char(' ') if ctrl => "\0".into(),
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

    #[test]
    fn mouse_modes_preserve_sgr_coordinates_and_release() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind as Kind};
        let mut parser = vt100::Parser::new(30, 120, 0);
        let event = MouseEvent {
            kind: Kind::Down(MouseButton::Left),
            column: 9,
            row: 4,
            modifiers: Mod::CONTROL,
        };
        assert!(mouse_bytes(event, parser.screen()).is_none());
        parser.process(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            mouse_bytes(event, parser.screen()).unwrap(),
            b"\x1b[<16;10;5M"
        );
        assert_eq!(
            mouse_bytes(
                MouseEvent {
                    kind: Kind::Up(MouseButton::Left),
                    ..event
                },
                parser.screen()
            )
            .unwrap(),
            b"\x1b[<16;10;5m"
        );
        assert!(
            mouse_bytes(
                MouseEvent {
                    kind: Kind::Moved,
                    ..event
                },
                parser.screen()
            )
            .is_none()
        );
        parser.process(b"\x1b[?1003h");
        assert!(
            mouse_bytes(
                MouseEvent {
                    kind: Kind::Moved,
                    ..event
                },
                parser.screen()
            )
            .is_some()
        );
        parser.process(b"\x1b[?1003l\x1b[?1000l");
        assert!(mouse_bytes(event, parser.screen()).is_none());
    }
}
