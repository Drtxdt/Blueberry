use crossterm::{cursor::{Hide,Show},event::{DisableMouseCapture,EnableMouseCapture},execute,terminal::{self,EnterAlternateScreen,LeaveAlternateScreen}};
use std::io::{self,Write};
use unicode_width::{UnicodeWidthChar,UnicodeWidthStr};

pub struct ScreenGuard{mouse:bool}
impl ScreenGuard{pub fn enter(mouse:bool)->io::Result<Self>{terminal::enable_raw_mode()?;if mouse{execute!(io::stdout(),EnterAlternateScreen,Hide,EnableMouseCapture)?}else{execute!(io::stdout(),EnterAlternateScreen,Hide)?}Ok(Self{mouse})}}
impl Drop for ScreenGuard{fn drop(&mut self){if self.mouse{let _=execute!(io::stdout(),DisableMouseCapture,Show,LeaveAlternateScreen);}else{let _=execute!(io::stdout(),Show,LeaveAlternateScreen);}let _=terminal::disable_raw_mode();let _=io::stdout().flush();}}
pub fn fit(text:&str,width:usize)->String{if width==0{return String::new()}if UnicodeWidthStr::width(text)<=width{return text.into()}if width==1{return "…".into()}let mut out=String::new();let mut used=0;for ch in text.chars(){let w=UnicodeWidthChar::width(ch).unwrap_or(0);if used+w>width-1{break}out.push(ch);used+=w}out.push('…');out}
