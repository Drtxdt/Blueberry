use crate::{config::Config, menu, model::Candidate};
use std::io::{self, Write};

#[derive(Default)]
pub struct Overlay {
    rows: Option<(u16, u16)>,
    painted: Option<Painted>,
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
        self.painted = None;
        #[cfg(windows)]
        if self.native_mode {
            self.rows = None;
            if let Some(Err(error)) = self.native.as_ref().map(|native| native.close(output)) {
                self.native_failed(error);
            }
            return Ok(());
        }
        if let Some((top, height)) = self.rows.take() {
            let (_, cols) = screen.size();
            write!(output, "\x1b[?25l")?;
            for (index, row) in screen
                .rows_formatted(0, cols)
                .enumerate()
                .skip(top.into())
                .take(height.into())
            {
                write!(output, "\x1b[{};1H\x1b[0m\x1b[2K", index + 1)?;
                output.write_all(&row)?;
            }
            restore_cursor(screen, output)?;
        }
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
        let below = rows.saturating_sub(cursor_row + 1);
        let above = cursor_row;
        let available = below.max(above);
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
            cursor_row + 1
        } else {
            cursor_row - height
        };
        let left = cursor_col.min(cols.saturating_sub(frame.width + 1));
        let painted = Painted {
            top,
            left,
            size: (rows, cols),
            cursor: (cursor_row, cursor_col),
            frame,
        };
        if self.painted.as_ref() == Some(&painted) {
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
            previous.top == painted.top
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
