//! Reading PSReadLine history files into the database.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{self, Result};

/// How many bytes from the start of a file are kept to detect a rewrite.
const HEAD_BYTES: usize = 4096;

/// What happened to one history file during an import.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Never seen before: read from the start.
    New,
    /// Same file, grown since last time: read only the new part.
    Appended,
    /// The start of the file changed (cleared or replaced): re-read it all,
    /// skipping commands already stored for this file.
    Rewritten,
    /// Nothing new since the last import.
    Unchanged,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::New => "new",
            Mode::Appended => "appended",
            Mode::Rewritten => "rewritten",
            Mode::Unchanged => "unchanged",
        }
    }
}

pub struct FileReport {
    pub path: PathBuf,
    pub mode: Mode,
    pub entries: usize,
    pub added: usize,
}

struct Entry {
    command: String,
    line_number: i64,
}

/// Every *_history.txt file in the PSReadLine folders for this OS.
///
/// This picks up ConsoleHost_history.txt and also histories from other hosts,
/// such as "Visual Studio Code Host_history.txt".
pub fn default_history_files() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let base = PathBuf::from(appdata).join("Microsoft");
            dirs.push(base.join("Windows").join("PowerShell").join("PSReadLine"));
            dirs.push(base.join("PowerShell").join("PSReadLine"));
        }
    } else {
        let data_home = std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".local").join("share"))
            });
        if let Some(base) = data_home {
            dirs.push(base.join("powershell").join("PSReadLine"));
        }
    }

    let mut files = Vec::new();
    for dir in dirs {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<PathBuf> = listing
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.to_ascii_lowercase().ends_with("_history.txt"))
                        .unwrap_or(false)
            })
            .collect();
        found.sort();
        files.extend(found);
    }
    files
}

/// Split raw history bytes into commands.
///
/// PSReadLine writes an embedded newline as a backtick at the end of a line,
/// so a line ending in ` continues onto the next one. Only complete lines
/// (ending in \n) are used; a partial last line is left for the next import.
///
/// Returns the commands and how many bytes were fully consumed.
fn parse(data: &[u8], first_line: i64) -> (Vec<Entry>, usize) {
    let mut entries = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut line_number = first_line;
    let mut start_line = first_line;
    let mut pos = 0;
    let mut consumed = 0;

    while let Some(newline) = data[pos..].iter().position(|&byte| byte == b'\n') {
        let raw = &data[pos..pos + newline];
        pos += newline + 1;
        line_number += 1;

        let mut line = String::from_utf8_lossy(raw).into_owned();
        if line.ends_with('\r') {
            line.pop();
        }
        if line.ends_with('`') {
            line.pop();
            pending.push(line);
            continue;
        }

        pending.push(line);
        let command = pending.join("\n");
        pending.clear();
        consumed = pos;
        if !command.trim().is_empty() {
            entries.push(Entry {
                command,
                line_number: start_line,
            });
        }
        start_line = line_number;
    }
    (entries, consumed)
}

/// Import one history file for the given host.
pub fn import_file(conn: &mut Connection, host_id: i64, path: &Path) -> Result<FileReport> {
    let data = std::fs::read(path)?;
    let source = path.display().to_string();
    let head = &data[..data.len().min(HEAD_BYTES)];

    let state: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT bytes_imported, head FROM import_state WHERE host_id = ?1 AND source = ?2",
            params![host_id, source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let (mode, offset) = match state {
        None => (Mode::New, 0),
        Some((done, old_head)) => {
            let done = done as usize;
            let same_start = data.len() >= old_head.len() && data[..old_head.len()] == old_head[..];
            if same_start && done == data.len() {
                return Ok(FileReport {
                    path: path.to_path_buf(),
                    mode: Mode::Unchanged,
                    entries: 0,
                    added: 0,
                });
            }
            if same_start && done < data.len() {
                (Mode::Appended, done)
            } else {
                (Mode::Rewritten, 0)
            }
        }
    };

    // Skip a UTF-8 byte-order mark at the very start of the file.
    let start = if offset == 0 && data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        offset
    };
    let first_line = data[..start].iter().filter(|&&byte| byte == b'\n').count() as i64 + 1;
    let (entries, consumed) = parse(&data[start..], first_line);
    let new_offset = start + consumed;

    let stamp = db::now_stamp();
    let tx = conn.transaction()?;
    let mut added = 0;
    {
        let sql = match mode {
            Mode::Rewritten => {
                "INSERT INTO history(host_id, command, source, line_number, imported_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (
                     SELECT 1 FROM history WHERE host_id = ?1 AND source = ?3 AND command = ?2
                 )"
            }
            _ => {
                "INSERT OR IGNORE INTO history(host_id, command, source, line_number, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            }
        };
        let mut stmt = tx.prepare(sql)?;
        for entry in &entries {
            added += stmt.execute(params![host_id, entry.command, source, entry.line_number, stamp])?;
        }
    }
    tx.execute(
        "INSERT INTO import_state(host_id, source, bytes_imported, head) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(host_id, source) DO UPDATE SET
             bytes_imported = excluded.bytes_imported,
             head = excluded.head",
        params![host_id, source, new_offset as i64, head],
    )?;
    tx.commit()?;

    Ok(FileReport {
        path: path.to_path_buf(),
        mode,
        entries: entries.len(),
        added,
    })
}

/// Record that this host was just imported.
pub fn mark_imported(conn: &Connection, host_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE hosts SET last_import = ?1 WHERE id = ?2",
        params![db::now_stamp(), host_id],
    )?;
    Ok(())
}