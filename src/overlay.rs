use crate::{config::Config, menu, model::Candidate};
use std::io::{self, Write};

#[derive(Default)]
pub struct Overlay {
    rows: Option<(u16, u16)>,
    painted: Option<Painted>,
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
    pub fn erase(&mut self, screen: &vt100::Screen, output: &mut impl Write) -> io::Result<()> {
        self.painted = None;
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
        let mut config = config.clone();
        let border_rows = if config.ui.border == "none" { 0 } else { 2 };
        config.ui.max_rows = config
            .ui
            .max_rows
            .min(usize::from(available).saturating_sub(border_rows))
            .max(1);
        let frame = menu::render(candidates, selected, query, cols - 1, &config);
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
