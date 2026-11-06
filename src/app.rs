//! The interactive application: main menu, dialogs, and actions.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::db;
use crate::import;
use crate::input::{self, Input, Key};
use crate::ui::{self, Screen, DLG_BANNER, DLG_BORDER, DLG_HOT, DLG_TEXT, DLG_TITLE, MENU_SEL, MENU_SEL_HOT};

/// What is drawn behind a dialog. Each screen of the app gets a variant here.
pub enum View {
    /// The main menu, with this MENU index highlighted.
    Menu(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Browse,
    Hosts,
    Search,
    Import,
    Info,
    Quit,
}

/// Main-menu entries; ~X~ marks the hotkey, None draws a divider.
const MENU: &[Option<(&str, Action)>] = &[
    Some(("~B~rowse history", Action::Browse)),
    Some(("Browse by ~h~ost", Action::Hosts)),
    Some(("~S~earch history…", Action::Search)),
    Some(("~R~e-import history files", Action::Import)),
    Some(("Database ~i~nfo", Action::Info)),
    None,
    Some(("E~x~it", Action::Quit)),
];

/// The character between the first pair of ~ markers, lowercased.
fn hotkey_of(label: &str) -> Option<char> {
    label
        .split('~')
        .nth(1)
        .and_then(|part| part.chars().next())
        .map(|ch| ch.to_ascii_lowercase())
}

fn hotkey_action(ch: char) -> Option<Action> {
    match ch.to_ascii_lowercase() {
        '/' => Some(Action::Search),
        'q' => Some(Action::Quit),
        lower => MENU
            .iter()
            .flatten()
            .find(|item| hotkey_of(item.0) == Some(lower))
            .map(|item| item.1),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub struct App {
    conn: Connection,
    db_path: PathBuf,
    host_id: i64,
    host_label: String,
    files: Vec<PathBuf>,
    status: String,
    last_size: Option<(i32, i32)>,
    quit: bool,
}

impl App {
    pub fn new(
        conn: Connection,
        db_path: PathBuf,
        host_id: i64,
        host_label: String,
        files: Vec<PathBuf>,
        status: String,
    ) -> App {
        App {
            conn,
            db_path,
            host_id,
            host_label,
            files,
            status,
            last_size: None,
            quit: false,
        }
    }

    /// Enter full-screen mode and run until the user exits.
    pub fn run(mut self) -> db::Result<()> {
        let _terminal = ui::Terminal::enter()?;
        self.main_menu()
    }

    /// Start a new frame, clearing the screen first if its size changed.
    fn frame(&mut self) -> Screen {
        let mut scr = Screen::new();
        if self.last_size != Some((scr.width, scr.height)) {
            scr.clear();
            self.last_size = Some((scr.width, scr.height));
        }
        scr
    }

    /// Block for a key. Returns None when the screen needs redrawing
    /// (resize or clock tick). Ctrl+C anywhere asks the whole app to quit.
    fn wait_key(&mut self) -> db::Result<Option<Key>> {
        let clock = ui::clock();
        loop {
            match input::read(Duration::from_millis(500))? {
                Input::Key(Key::CtrlC) => {
                    self.quit = true;
                    return Ok(Some(Key::Esc));
                }
                Input::Key(key) => return Ok(Some(key)),
                Input::Resize => return Ok(None),
                Input::Timeout => {
                    if ui::clock() != clock {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn scalar(&self, sql: &str) -> db::Result<i64> {
        Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
    }

    /// Draw whatever a View describes; used for screens and for dialog backgrounds.
    fn draw_view(&self, scr: &mut Screen, view: &View) -> db::Result<()> {
        match view {
            View::Menu(selected) => self.draw_menu(scr, *selected),
        }
    }

    // ── dialogs ──

    fn message_box(&mut self, title: &str, lines: &[String], under: &View) -> db::Result<()> {
        loop {
            if self.quit {
                return Ok(());
            }
            let mut scr = self.frame();
            self.draw_view(&mut scr, under)?;
            if !scr.blocked {
                let longest = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0) as i32;
                let w = (scr.width - 8).min((longest + 6).max(44));
                let options = textwrap::Options::new((w - 6).max(1) as usize)
                    .word_splitter(textwrap::WordSplitter::NoHyphenation);
                let mut wrapped: Vec<String> = Vec::new();
                for line in lines {
                    let parts = textwrap::wrap(line, &options);
                    if parts.is_empty() {
                        wrapped.push(String::new());
                    } else {
                        wrapped.extend(parts.into_iter().map(|part| part.into_owned()));
                    }
                }
                wrapped.truncate((scr.height - 10).max(1) as usize);

                let h = wrapped.len() as i32 + 6;
                let (top, left) = ui::centered(&scr, h, w);
                ui::draw_box(&mut scr, top, left, h, w, DLG_BORDER, title, DLG_TITLE);
                for (i, line) in wrapped.iter().enumerate() {
                    scr.at(top + 2 + i as i32, left + 3, line, DLG_TEXT);
                }
                ui::draw_buttons(&mut scr, top + h - 3, left, w, &["~O~K"], 0);
                ui::status_bar(&mut scr, "~Enter~ Close");
            }
            scr.flush(None)?;
            match self.wait_key()? {
                Some(Key::Enter) | Some(Key::Esc) | Some(Key::Char(' ')) | Some(Key::Char('o'))
                | Some(Key::Char('O')) => return Ok(()),
                _ => {}
            }
        }
    }

    // ── actions ──

    fn reimport(&mut self, under: &View) -> db::Result<()> {
        let mut total = 0;
        let mut details = Vec::new();
        for path in &self.files {
            match import::import_file(&mut self.conn, self.host_id, path) {
                Ok(report) => {
                    total += report.added;
                    details.push(format!(
                        "{}: {} new ({})",
                        file_name(path),
                        ui::thousands(report.added as i64),
                        report.mode.label()
                    ));
                }
                Err(error) => details.push(format!("{}: {}", file_name(path), error)),
            }
        }
        if self.files.is_empty() {
            details.push("No history files were found.".to_string());
        } else {
            import::mark_imported(&self.conn, self.host_id)?;
        }
        self.status = format!("{} new entries imported", ui::thousands(total as i64));

        let mut lines = vec![
            format!("Added {} new entries for {}.", ui::thousands(total as i64), self.host_label),
            String::new(),
        ];
        lines.extend(details);
        self.message_box("Re-import", &lines, under)
    }

    fn show_info(&mut self, under: &View) -> db::Result<()> {
        let mut lines = vec![
            format!("Database:  {}", self.db_path.display()),
            format!("This host: {}", self.host_label),
            format!("Entries:   {}", ui::thousands(self.scalar("SELECT COUNT(*) FROM history")?)),
            String::new(),
            "Hosts:".to_string(),
        ];
        let hosts = db::list_hosts(&self.conn)?;
        if hosts.is_empty() {
            lines.push("  (none)".to_string());
        }
        for h in hosts {
            lines.push(format!(
                "  {:>7}  {}\\{}  (last import {})",
                ui::thousands(h.commands),
                h.hostname,
                h.username,
                h.last_import.as_deref().unwrap_or("never")
            ));
        }
        self.message_box("Database Info", &lines, under)
    }

    // ── main menu ──

    fn draw_menu(&self, scr: &mut Screen, selected: usize) -> db::Result<()> {
        ui::desktop(scr, "~↑↓~ Select  ~Enter~ Choose  ~X~ Exit", &self.status);
        if scr.blocked {
            return Ok(());
        }
        let w = 38;
        let h = MENU.len() as i32 + 7;
        let (top, left) = ui::centered(scr, h, w);
        let inner = (w - 2) as usize;

        ui::draw_box(scr, top, left, h, w, DLG_BORDER, "Main Menu", DLG_TITLE);
        scr.at(top + 1, left + 1, &format!("{:^inner$}", "▓▒░  P S - H I S T  ░▒▓"), DLG_BANNER);
        scr.at(top + 2, left + 1, &format!("{:^inner$}", "PowerShell Command History"), DLG_TEXT);
        ui::separator(scr, top + 3, left, w, DLG_BORDER);

        let mut row = top + 4;
        for (i, item) in MENU.iter().enumerate() {
            match item {
                None => ui::separator(scr, row, left, w, DLG_BORDER),
                Some((label, _)) => {
                    let (style, accent) = if i == selected {
                        (MENU_SEL, MENU_SEL_HOT)
                    } else {
                        (DLG_TEXT, DLG_HOT)
                    };
                    scr.at(row, left + 2, &" ".repeat((w - 4) as usize), style);
                    ui::hot(scr, row, left + 2, &format!("  {}", label), style, accent);
                }
            }
            row += 1;
        }

        ui::separator(scr, row, left, w, DLG_BORDER);
        let commands = self.scalar("SELECT COUNT(*) FROM history")?;
        let hosts = self.scalar("SELECT COUNT(*) FROM hosts")?;
        let summary = format!(
            "{} commands · {} host{}",
            ui::thousands(commands),
            hosts,
            if hosts == 1 { "" } else { "s" }
        );
        scr.at(row + 1, left + 1, &format!("{:^inner$}", summary), DLG_TEXT);
        Ok(())
    }

    fn main_menu(&mut self) -> db::Result<()> {
        // Indexes of the selectable (non-divider) entries.
        let choices: Vec<usize> = MENU
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_some())
            .map(|(i, _)| i)
            .collect();
        let mut pos = 0;

        loop {
            if self.quit {
                return Ok(());
            }
            let mut scr = self.frame();
            self.draw_view(&mut scr, &View::Menu(choices[pos]))?;
            scr.flush(None)?;

            let Some(key) = self.wait_key()? else {
                continue;
            };
            let mut action = None;
            match key {
                Key::Up => pos = (pos + choices.len() - 1) % choices.len(),
                Key::Down => pos = (pos + 1) % choices.len(),
                Key::Home => pos = 0,
                Key::End => pos = choices.len() - 1,
                Key::Enter => action = MENU[choices[pos]].map(|item| item.1),
                Key::Char(ch) => {
                    if let Some(found) = hotkey_action(ch) {
                        action = Some(found);
                        if let Some(p) = choices
                            .iter()
                            .position(|&i| MENU[i].map(|item| item.1) == Some(found))
                        {
                            pos = p;
                        }
                    }
                }
                _ => {}
            }

            let under = View::Menu(choices[pos]);
            match action {
                Some(Action::Quit) => return Ok(()),
                Some(Action::Import) => self.reimport(&under)?,
                Some(Action::Info) => self.show_info(&under)?,
                Some(Action::Browse) | Some(Action::Hosts) | Some(Action::Search) => {
                    let lines = vec!["This part of the port comes in a later step.".to_string()];
                    self.message_box("Not Yet", &lines, &under)?;
                }
                None => {}
            }
        }
    }
}