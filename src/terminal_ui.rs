use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};

pub struct ScreenGuard {
    mouse: bool,
}
impl ScreenGuard {
    pub fn enter(mouse: bool) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        if mouse {
            execute!(io::stdout(), EnterAlternateScreen, Hide, EnableMouseCapture)?
        } else {
            execute!(io::stdout(), EnterAlternateScreen, Hide)?
        }
        Ok(Self { mouse })
    }
}
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(
                io::stdout(),
                DisableMouseCapture,
                Show,
                LeaveAlternateScreen
            );
        } else {
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        }
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().flush();
    }
}
pub fn fit(text: &str, width: usize) -> String {
    crate::menu::truncate_to_width(&crate::menu::sanitize_text(text), width)
}
