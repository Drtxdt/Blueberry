use crate::{config::Config, menu, model::Candidate};
use std::io::{self, Write};

#[derive(Default)]
pub struct Overlay {
    rows: Option<(u16, u16)>,
    painted: Option<Painted>,
    menu_row: Option<u16>,
    resized: bool,
    #[cfg(windows)]
    native: Option<crate::native_overlay::NativeOverlay>,
    #[cfg(windows)]
    native_mode: bool,
    #[cfg(windows)]
    native_retry: bool,
    #[cfg(windows)]
    native_error: Option<String>,
}

#[derive(PartialEq, Eq)]
struct Painted {
    top: u16,
    left: u16,
    size: (u16, u16),
    cursor: (u16, u16),
    frame: menu::MenuFrame,
}

impl Overlay {
    pub fn set_menu_row(&mut self, row: Option<u16>) {
        self.menu_row = row;
    }
    pub fn resized(&mut self) {
        self.menu_row = None;
        self.resized = true;
    }
    #[cfg(windows)]
    pub fn with_native(client: crate::native_overlay::NativeOverlay) -> Self {
        Self {
            native: Some(client),
            native_mode: true,
            ..Self::default()
        }
    }

    pub fn is_native(&self) -> bool {
        #[cfg(windows)]
        {
            self.native_mode
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn reset_layout(&mut self) {
        self.rows = None;
        self.painted = None;
        self.menu_row = None;
        self.resized = false;
    }

    pub fn new_prompt(&mut self) {
        #[cfg(windows)]
        if self.native_mode && self.native.is_none() {
            self.native_retry = true;
        }
    }

    pub fn frame_visible(&self) -> bool {
        self.rows.is_some() && self.painted.is_some()
    }

    /// Ownership readback must see preceding shell output before it decides
    /// which physical rows still belong to the menu. Otherwise an erase queued
    /// after that output can delete a new prompt using stale character matches.
    pub fn synchronize_pending_output(
        &self,
        pending: &mut Vec<u8>,
        output: &mut impl Write,
    ) -> io::Result<()> {
        if self.frame_visible() && !pending.is_empty() {
            output.write_all(pending)?;
            output.flush()?;
            pending.clear();
        }
        Ok(())
    }

    #[cfg(windows)]
    fn native_failed(&mut self, error: io::Error) {
        self.native_error = Some(error.to_string());
        self.native = None;
        self.reset_layout();
    }

    pub fn fallback_to_ordinary(&mut self) {
        #[cfg(windows)]
        {
            self.native = None;
            self.native_mode = false;
            self.native_retry = false;
        }
        self.reset_layout();
    }

    pub fn take_native_error(&mut self) -> Option<String> {
        #[cfg(windows)]
        {
            self.native_error.take()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// Must receive the actual output writer, after queued shell bytes have
    /// been flushed. A Vec cannot deliver the synchronous retirement fence.
    pub fn close_for_input(&mut self, output: &mut impl Write) -> bool {
        #[cfg(windows)]
        if self.native_mode {
            self.reset_layout();
            match self.native.as_ref().map(|native| native.close(output)) {
                Some(Ok(Some(true))) => return true,
                Some(Err(error)) => self.native_failed(error),
                _ => {}
            }
            return false;
        }
        #[cfg(not(windows))]
        let _ = output;
        true
    }

    pub fn erase(&mut self, screen: &vt100::Screen, output: &mut impl Write) -> io::Result<()> {
        let painted = self.painted.take();
        #[cfg(windows)]
        if self.native_mode {
            self.rows = None;
            self.resized = false;
            if let Some(Err(error)) = self.native.as_ref().map(|native| native.close(output)) {
                self.native_failed(error);
            }
            return Ok(());
        }
        if let Some((top, height)) = self.rows.take() {
            write!(output, "\x1b[?25l")?;
            let owned = painted
                .as_ref()
                .map(|p| owned_console_rows(p, screen.size(), self.resized));
            let (viewport_height, owned) = owned.unwrap_or_else(|| {
                (
                    screen.size().0,
                    (top..top.saturating_add(height).min(screen.size().0)).collect(),
                )
            });
            for row in owned {
                write!(output, "\x1b[{};1H\x1b[0m\x1b[2K", row + 1)?;
                finish_owned_row(output, row, viewport_height)?;
            }
            restore_cursor(screen, output)?;
        }
        self.resized = false;
        Ok(())
    }

    pub fn draw(
        &mut self,
        screen: &vt100::Screen,
        output: &mut impl Write,
        candidates: &[Candidate],
        selected: usize,
        query: &str,
        config: &Config,
    ) -> io::Result<()> {
        self.draw_with_state(
            screen,
            output,
            candidates,
            selected,
            query,
            config,
            menu::MenuState::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_state(
        &mut self,
        screen: &vt100::Screen,
        output: &mut impl Write,
        candidates: &[Candidate],
        selected: usize,
        query: &str,
        config: &Config,
        state: menu::MenuState,
    ) -> io::Result<()> {
        if candidates.is_empty() || screen.alternate_screen() {
            return self.erase(screen, output);
        }
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        if rows < 4 || cols < 8 {
            return self.erase(screen, output);
        }
        let menu_row = self.menu_row.unwrap_or(cursor_row + 1).max(cursor_row + 1);
        let below = rows.saturating_sub(menu_row);
        let above = cursor_row;
        let available = if self.is_native() {
            below.max(above)
        } else {
            below
        };
        if available < 3 {
            return self.erase(screen, output);
        }
        let mut config = Config {
            ui: config.ui.clone(),
            completion: config.completion.clone(),
            descriptions: Default::default(),
            ..Default::default()
        };
        let border_rows = if config.ui.border == "none" { 0 } else { 2 };
        config.ui.max_rows = config
            .ui
            .max_rows
            .min(usize::from(available).saturating_sub(border_rows))
            .max(1);
        let frame = menu::render_with_state(
            candidates,
            selected,
            query,
            cols - 1,
            &config,
            state,
            available.into(),
        );
        let height = frame.lines.len() as u16;
        if height > available || height == 0 {
            return self.erase(screen, output);
        }
        let top = if below >= height {
            menu_row
        } else {
            cursor_row - height
        };
        let left = if self.is_native() {
            cursor_col.min(cols.saturating_sub(frame.width + 1))
        } else {
            0
        };
        let painted = Painted {
            top,
            left,
            size: (rows, cols),
            cursor: (cursor_row, cursor_col),
            frame,
        };
        if !self.resized && self.painted.as_ref() == Some(&painted) {
            return Ok(());
        }
        #[cfg(windows)]
        if self.native_mode {
            let reconnect = self
                .native
                .as_ref()
                .is_some_and(|native| !native.is_connected())
                || self.native_retry;
            self.erase(screen, output)?;
            // A child Blueberry session can temporarily take ownership of the
            // same terminal. Reconnect once when the parent becomes active.
            if reconnect {
                self.native = None;
                self.native_retry = false;
                match crate::native_overlay::NativeOverlay::connect(output) {
                    Ok(native) => self.native = Some(native),
                    Err(error) => self.native_failed(error),
                }
            }
            if let Some(native) = &mut self.native {
                match native.paint(output, &painted.frame.lines, top, left, painted.frame.width) {
                    Ok(true) => {
                        self.rows = Some((top, height));
                        self.painted = Some(painted);
                    }
                    Ok(false) => {}
                    Err(error) => self.native_failed(error),
                }
            }
            return Ok(());
        }
        if let Some(previous) = self.painted.as_ref().filter(|previous| {
            !self.resized
                && previous.top == painted.top
                && previous.left == painted.left
                && previous.size == painted.size
                && previous.frame.width == painted.frame.width
                && previous.frame.lines.len() == painted.frame.lines.len()
        }) {
            write!(output, "\x1b[?25l")?;
            for (i, (before, after)) in previous
                .frame
                .lines
                .iter()
                .zip(&painted.frame.lines)
                .enumerate()
            {
                if before != after {
                    write!(
                        output,
                        "\x1b[{};{}H\x1b[0m{}",
                        usize::from(top) + i + 1,
                        left + 1,
                        after
                    )?;
                    finish_owned_row(output, top + i as u16, rows)?;
                }
            }
            restore_cursor(screen, output)?;
            self.painted = Some(painted);
            return Ok(());
        }
        self.erase(screen, output)?;
        write!(output, "\x1b[?25l")?;
        for (i, row) in painted.frame.lines.iter().enumerate() {
            write!(
                output,
                "\x1b[{};{}H\x1b[0m{}",
                usize::from(top) + i + 1,
                left + 1,
                row
            )?;
            finish_owned_row(output, top + i as u16, rows)?;
        }
        restore_cursor(screen, output)?;
        self.rows = Some((top, height));
        self.painted = Some(painted);
        Ok(())
    }
}

fn restore_cursor(screen: &vt100::Screen, output: &mut impl Write) -> io::Result<()> {
    output.write_all(&screen.cursor_state_formatted())?;
    output.write_all(&screen.attributes_formatted())
}

fn finish_owned_row(output: &mut impl Write, row: u16, viewport_height: u16) -> io::Result<()> {
    // EL clears characters, but retains ConPTY's forced-wrap metadata.
    // An explicit LF marks our row as a hard line so a later widening cannot
    // concatenate a repainted menu with its neighbour. Never scroll the bottom.
    if row + 1 < viewport_height {
        output.write_all(b"\r\n")?;
    }
    Ok(())
}

/// Inspect only character ownership. Historical cells are never replayed from
/// CHAR_INFO or the VT model; their Unicode and RGB remain terminal-owned.
fn owned_console_rows(painted: &Painted, size: (u16, u16), resized: bool) -> (u16, Vec<u16>) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            CHAR_INFO, CONSOLE_SCREEN_BUFFER_INFO, COORD, GetConsoleScreenBufferInfo, GetStdHandle,
            ReadConsoleOutputW, SMALL_RECT, STD_OUTPUT_HANDLE,
        };
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } != 0 {
            let width = (info.srWindow.Right - info.srWindow.Left + 1).max(1);
            let height = (info.srWindow.Bottom - info.srWindow.Top + 1).max(1) as u16;
            // Console reflow precedes delivery of the resize input event.
            // Shell output can ask us to erase in that interval, while the
            // parser and `resized` still describe the old viewport.
            let resized = resized || painted.size != (height, width as u16);
            let mut owned = Vec::new();
            for line in &painted.frame.lines {
                let mut parser = vt100::Parser::new(1, painted.size.1, 0);
                parser.process(line.as_bytes());
                let text = parser.screen().contents();
                owned.push(text.trim().to_owned());
                if resized {
                    use unicode_segmentation::UnicodeSegmentation;
                    use unicode_width::UnicodeWidthStr;
                    let mut part = String::new();
                    let mut cells = 0;
                    for glyph in text.graphemes(true) {
                        let next = UnicodeWidthStr::width(glyph);
                        if cells + next > width as usize {
                            owned.push(part.trim().to_owned());
                            part.clear();
                            cells = 0;
                        }
                        part.push_str(glyph);
                        cells += next;
                    }
                    owned.push(part.trim().to_owned());
                }
            }
            let mut result = Vec::new();
            #[cfg(debug_assertions)]
            let audit_path =
                std::env::var_os("BLUEBERRY_TEST_OVERLAY_AUDIT").filter(|path| !path.is_empty());
            #[cfg(debug_assertions)]
            let mut audit_rows = Vec::new();
            let first = if resized { 0 } else { painted.top };
            let end = if resized {
                height
            } else {
                painted
                    .top
                    .saturating_add(painted.frame.lines.len() as u16)
                    .min(size.0)
            };
            for y in first..end {
                let mut area = SMALL_RECT {
                    Left: info.srWindow.Left,
                    Right: info.srWindow.Right,
                    Top: info.srWindow.Top + y as i16,
                    Bottom: info.srWindow.Top + y as i16,
                };
                let mut cells = vec![CHAR_INFO::default(); width as usize];
                if unsafe {
                    ReadConsoleOutputW(
                        handle,
                        cells.as_mut_ptr(),
                        COORD { X: width, Y: 1 },
                        COORD { X: 0, Y: 0 },
                        &mut area,
                    )
                } == 0
                {
                    continue;
                }
                let text = String::from_utf16_lossy(
                    &cells
                        .iter()
                        .filter(|cell| {
                            cell.Attributes & 0x0200 == 0
                                || unsafe { (0xdc00..=0xdfff).contains(&cell.Char.UnicodeChar) }
                        })
                        .map(|cell| unsafe { cell.Char.UnicodeChar })
                        .collect::<Vec<_>>(),
                );
                let text = text.trim_matches([' ', '\0']);
                let matched = !text.is_empty()
                    && owned.iter().any(|line| {
                        line == text
                            || (text.contains('\u{fffd}')
                                && line
                                    .chars()
                                    .map(|ch| if ch as u32 > 0xffff { '\u{fffd}' } else { ch })
                                    .collect::<String>()
                                    == text)
                    });
                #[cfg(debug_assertions)]
                if audit_path.is_some() {
                    audit_rows.push(serde_json::json!({"row":y,"text":text,"matched":matched}));
                }
                if matched {
                    result.push(y);
                }
            }
            #[cfg(debug_assertions)]
            if let Some(path) = audit_path {
                // Explicit diagnostic fixtures only. A logging failure must
                // never change completion or menu retirement behavior.
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let record = serde_json::json!({
                        "event":"overlay_ownership", "resized":resized,
                        "painted_size":painted.size,"model_size":size,
                        "viewport":[height,width],"window_top":info.srWindow.Top,
                        "painted_top":painted.top,"painted_cursor":painted.cursor,
                        "native_cursor":[info.dwCursorPosition.Y,info.dwCursorPosition.X],
                        "owned_text":owned,"read_rows":audit_rows,"erase_rows":result
                    });
                    let _ = writeln!(file, "{record}");
                }
            }
            return (height, result);
        }
    }
    let _ = resized;
    (
        size.0,
        (painted.top
            ..painted
                .top
                .saturating_add(painted.frame.lines.len() as u16)
                .min(size.0))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn resizing_through_a_wide_character_keeps_the_parser_writable() {
        for alternate in [false, true] {
            for width in 1..6 {
                let mut parser = vt100::Parser::new(2, 6, 0);
                if alternate {
                    parser.process(b"\x1b[?1049h");
                }
                parser.process("中中中".as_bytes());
                parser.screen_mut().set_size(2, width);
                parser.process(format!("\x1b[1;{width}HX").as_bytes());
                assert_eq!(parser.screen().cell(0, width - 1).unwrap().contents(), "X");
            }
        }
    }
    use super::*;
    use crate::model::CandidateKind;
    #[test]
    fn unchanged_menu_writes_nothing_but_selection_and_position_redraw() {
        let mut shell = vt100::Parser::new(24, 80, 0);
        shell.process(b"PS> gi");
        let candidates: Vec<_> = ["git", "gitk"]
            .into_iter()
            .map(|name| Candidate {
                label: name.into(),
                insert_text: name.into(),
                description: "source control".into(),
                kind: CandidateKind::Command,
                ..Default::default()
            })
            .collect();
        let mut overlay = Overlay::default();
        let mut bytes = Vec::new();
        overlay
            .draw(
                shell.screen(),
                &mut bytes,
                &candidates,
                0,
                "gi",
                &Config::default(),
            )
            .unwrap();
        assert!(!bytes.is_empty());
        bytes.clear();
        overlay
            .draw(
                shell.screen(),
                &mut bytes,
                &candidates,
                0,
                "gi",
                &Config::default(),
            )
            .unwrap();
        assert!(bytes.is_empty());
        overlay
            .draw(
                shell.screen(),
                &mut bytes,
                &candidates,
                1,
                "gi",
                &Config::default(),
            )
            .unwrap();
        assert!(!bytes.is_empty());
        bytes.clear();
        shell.process(b"\r\nPS> gi");
        overlay
            .draw(
                shell.screen(),
                &mut bytes,
                &candidates,
                1,
                "gi",
                &Config::default(),
            )
            .unwrap();
        assert!(!bytes.is_empty());
        bytes.clear();
        overlay
            .draw(shell.screen(), &mut bytes, &[], 0, "", &Config::default())
            .unwrap();
        assert!(!bytes.is_empty());
    }
    #[test]
    fn erasing_overlay_restores_terminal_contents_cursor_and_colors() {
        let mut original = vt100::Parser::new(24, 80, 0);
        original.process(b"old output\r\n\x1b[32mPS> gi");
        let mut visible = vt100::Parser::new(24, 80, 0);
        visible.process(&original.screen().state_formatted());
        let c = Candidate {
            label: "git".into(),
            insert_text: "git".into(),
            description: "source control".into(),
            kind: CandidateKind::Command,
            ..Default::default()
        };
        let mut overlay = Overlay::default();
        let mut bytes = Vec::new();
        overlay
            .draw(
                original.screen(),
                &mut bytes,
                &[c],
                0,
                "gi",
                &Config::default(),
            )
            .unwrap();
        visible.process(&bytes);
        assert!(visible.screen().contents().contains("git"));
        bytes.clear();
        overlay.erase(original.screen(), &mut bytes).unwrap();
        visible.process(&bytes);
        assert_eq!(visible.screen().contents(), original.screen().contents());
        assert_eq!(
            visible.screen().cursor_position(),
            original.screen().cursor_position()
        );
        assert_eq!(
            visible.screen().attributes_formatted(),
            original.screen().attributes_formatted()
        );
    }
}
