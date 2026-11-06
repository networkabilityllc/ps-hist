//! Terminal drawing: colours, the frame buffer, and box-drawing helpers.

// Several colours and helpers are not used yet; they will be as the UI is ported.
#![allow(dead_code)]

use std::io::{self, Write};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen,
};

pub const RESET: &str = "\x1b[0m";

/// Build an SGR colour sequence at compile time, like sgr() in the Python version.
macro_rules! sgr {
    ($fg:literal, $bg:literal) => {
        concat!("\x1b[0;", $fg, ";", $bg, "m")
    };
}

// Classic 16-colour palette, in the spirit of mid-80s DOS menu shells.
pub const MENUBAR: &str = sgr!(30, 47);
pub const MENUBAR_HOT: &str = sgr!(31, 47);
pub const DESKTOP: &str = sgr!(94, 44);
pub const SHADOW: &str = sgr!(37, 40);
pub const WIN_BORDER: &str = sgr!(97, 44);
pub const WIN_TITLE: &str = sgr!(93, 44);
pub const WIN_TEXT: &str = sgr!(37, 44);
pub const WIN_DIM: &str = sgr!(36, 44);
pub const WIN_KEY: &str = sgr!(93, 44);
pub const WIN_SEL: &str = sgr!(30, 46);
pub const SCROLL: &str = sgr!(36, 44);
pub const SCROLL_THUMB: &str = sgr!(96, 44);
pub const DLG_BORDER: &str = sgr!(30, 47);
pub const DLG_TITLE: &str = sgr!(30, 47);
pub const DLG_TEXT: &str = sgr!(30, 47);
pub const DLG_HOT: &str = sgr!(31, 47);
pub const DLG_BANNER: &str = sgr!(34, 47);
pub const MENU_SEL: &str = sgr!(30, 42);
pub const MENU_SEL_HOT: &str = sgr!(93, 42);
pub const BTN: &str = sgr!(30, 42);
pub const BTN_FOCUS: &str = sgr!(97, 42);
pub const BTN_HOT: &str = sgr!(93, 42);
pub const BTN_SHADOW: &str = sgr!(30, 47);
pub const FIELD: &str = sgr!(97, 44);
pub const FIELD_KEY: &str = sgr!(93, 44);

pub const MIN_WIDTH: i32 = 50;
pub const MIN_HEIGHT: i32 = 16;

/// Puts the terminal into full-screen raw mode, and restores it when dropped.
///
/// Because restoring happens in Drop, the terminal is put back even if the
/// program returns early with an error or panics.
pub struct Terminal;

impl Terminal {
    pub fn enter() -> io::Result<Terminal> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            DisableLineWrap,
            Hide,
            Clear(ClearType::All)
        )?;
        Ok(Terminal)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = stdout.write_all(RESET.as_bytes());
        let _ = execute!(
            stdout,
            Clear(ClearType::All),
            EnableLineWrap,
            Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Collects one frame of positioned, styled text and writes it in a single burst.
pub struct Screen {
    pub width: i32,
    pub height: i32,
    pub blocked: bool,
    parts: String,
}

impl Screen {
    pub fn new() -> Screen {
        let (width, height) = terminal::size().unwrap_or((100, 28));
        Screen {
            width: width as i32,
            height: height as i32,
            blocked: false,
            parts: String::new(),
        }
    }

    /// Wipe the whole terminal before drawing this frame (used after a resize).
    pub fn clear(&mut self) {
        self.parts.push_str(RESET);
        self.parts.push_str("\x1b[2J");
    }

    /// Draw text at a 1-based row and column, clipped to the screen edges.
    pub fn at(&mut self, row: i32, col: i32, text: &str, style: &str) {
        if self.blocked || row < 1 || row > self.height {
            return;
        }
        let mut col = col;
        let mut skip = 0;
        if col < 1 {
            skip = (1 - col) as usize;
            col = 1;
        }
        let room = (self.width - col + 1).max(0) as usize;
        let visible: String = text.chars().skip(skip).take(room).collect();
        if !visible.is_empty() {
            self.parts
                .push_str(&format!("\x1b[{};{}H{}{}", row, col, style, visible));
        }
    }

    /// Write the frame. With a cursor position, show the cursor there; otherwise hide it.
    pub fn flush(&self, cursor: Option<(i32, i32)>) -> io::Result<()> {
        let mut out = String::with_capacity(self.parts.len() + 32);
        out.push_str(&self.parts);
        out.push_str(RESET);
        match cursor {
            Some((row, col)) if !self.blocked => {
                out.push_str(&format!("\x1b[{};{}H\x1b[?25h", row, col));
            }
            _ => out.push_str("\x1b[?25l"),
        }
        let mut stdout = io::stdout().lock();
        stdout.write_all(out.as_bytes())?;
        stdout.flush()
    }
}

pub fn clock() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Format a number with thousands separators: 4538 -> "4,538".
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{}", out)
    } else {
        out
    }
}

/// Visible width of hotkey markup, ignoring the ~ markers.
pub fn plain_len(markup: &str) -> i32 {
    markup.chars().filter(|&ch| ch != '~').count() as i32
}

/// Draw text where ~X~ marks a hotkey highlight. Returns the visible width.
pub fn hot(scr: &mut Screen, row: i32, col: i32, markup: &str, normal: &str, accent: &str) -> i32 {
    let start = col;
    let mut col = col;
    for (i, part) in markup.split('~').enumerate() {
        if !part.is_empty() {
            let style = if i % 2 == 1 { accent } else { normal };
            scr.at(row, col, part, style);
            col += part.chars().count() as i32;
        }
    }
    col - start
}

/// A double-line box with a drop shadow and an optional centred title.
pub fn draw_box(
    scr: &mut Screen,
    top: i32,
    left: i32,
    h: i32,
    w: i32,
    style: &str,
    title: &str,
    title_style: &str,
) {
    let inner = (w - 2).max(0) as usize;
    for row in top + 1..top + h + 1 {
        scr.at(row, left + w, "  ", SHADOW);
    }
    scr.at(top + h, left + 2, &" ".repeat(w.max(0) as usize), SHADOW);
    scr.at(top, left, &format!("╔{}╗", "═".repeat(inner)), style);
    let middle = format!("║{}║", " ".repeat(inner));
    for row in top + 1..top + h - 1 {
        scr.at(row, left, &middle, style);
    }
    scr.at(top + h - 1, left, &format!("╚{}╝", "═".repeat(inner)), style);
    if !title.is_empty() {
        let label: String = format!(" {} ", title)
            .chars()
            .take(inner.saturating_sub(2))
            .collect();
        let len = label.chars().count() as i32;
        let title_style = if title_style.is_empty() { style } else { title_style };
        scr.at(top, left + (w - len) / 2, &label, title_style);
    }
}

/// A single-line divider across a box, joined to its double-line sides.
pub fn separator(scr: &mut Screen, row: i32, left: i32, w: i32, style: &str) {
    let inner = (w - 2).max(0) as usize;
    scr.at(row, left, &format!("╟{}╢", "─".repeat(inner)), style);
}

/// Top-left corner that centres an h×w box on the screen.
pub fn centered(scr: &Screen, h: i32, w: i32) -> (i32, i32) {
    let top = ((scr.height - h) / 2 + 1).max(2);
    let left = ((scr.width - w) / 2 + 1).max(1);
    (top, left)
}

/// A centred row of push-buttons with shadows; `focus` is the highlighted one.
pub fn draw_buttons(scr: &mut Screen, row: i32, left: i32, w: i32, labels: &[&str], focus: usize) {
    let widths: Vec<i32> = labels.iter().map(|label| plain_len(label) + 3).collect();
    let total: i32 = widths.iter().sum::<i32>() + 2 * (labels.len() as i32 - 1);
    let mut col = left + (w - total) / 2;
    for (i, label) in labels.iter().enumerate() {
        let text = format!(" {} ", label);
        let width = plain_len(&text);
        let normal = if i == focus { BTN_FOCUS } else { BTN };
        hot(scr, row, col, &text, normal, BTN_HOT);
        scr.at(row, col + width, "▄", BTN_SHADOW);
        scr.at(row + 1, col + 1, &"▀".repeat(width as usize), BTN_SHADOW);
        col += widths[i] + 2;
    }
}

/// Menu bar, patterned desktop, and status bar with key hints.
pub fn desktop(scr: &mut Screen, hint_markup: &str, status: &str) {
    if scr.width < MIN_WIDTH || scr.height < MIN_HEIGHT {
        scr.parts.push_str(&format!(
            "{}\x1b[2J\x1b[1;1HWindow too small - enlarge to at least {}x{}.",
            RESET, MIN_WIDTH, MIN_HEIGHT
        ));
        scr.blocked = true;
        return;
    }
    let (w, h) = (scr.width, scr.height);
    scr.at(1, 1, &" ".repeat(w as usize), MENUBAR);
    hot(scr, 1, 2, "~≡~  PS-HIST  ·  PowerShell Command History", MENUBAR, MENUBAR_HOT);
    scr.at(1, w - 6, &clock(), MENUBAR);
    let fill = "░".repeat(w as usize);
    for row in 2..h {
        scr.at(row, 1, &fill, DESKTOP);
    }
    scr.at(h, 1, &" ".repeat(w as usize), MENUBAR);
    let used = hot(scr, h, 2, hint_markup, MENUBAR, MENUBAR_HOT) + 2;
    let status_len = status.chars().count() as i32;
    if !status.is_empty() && used + status_len + 4 < w {
        scr.at(h, w - status_len - 1, status, MENUBAR);
    }
}

/// Replace the status-bar hints (used while a dialog is open).
pub fn status_bar(scr: &mut Screen, hint_markup: &str) {
    if scr.blocked {
        return;
    }
    let (w, h) = (scr.width, scr.height);
    scr.at(h, 1, &" ".repeat(w as usize), MENUBAR);
    hot(scr, h, 2, hint_markup, MENUBAR, MENUBAR_HOT);
}