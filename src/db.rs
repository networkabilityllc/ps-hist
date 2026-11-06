//! SQLite storage: schema, hosts, and history rows.

use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Header written by the short-lived encrypted Python version.
const ENCRYPTED_MAGIC: &[u8] = b"PSHISTDB";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS hosts (
    id INTEGER PRIMARY KEY,
    hostname TEXT NOT NULL COLLATE NOCASE,
    username TEXT NOT NULL COLLATE NOCASE,
    first_seen TEXT NOT NULL,
    last_import TEXT,
    UNIQUE(hostname, username)
);

CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    command TEXT NOT NULL,
    source TEXT NOT NULL,
    line_number INTEGER NOT NULL,
    imported_at TEXT NOT NULL,
    UNIQUE(host_id, source, line_number, command)
);

CREATE TABLE IF NOT EXISTS import_state (
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    source TEXT NOT NULL,
    bytes_imported INTEGER NOT NULL,
    head BLOB NOT NULL,
    PRIMARY KEY(host_id, source)
);

CREATE INDEX IF NOT EXISTS history_host ON history(host_id, id);
CREATE INDEX IF NOT EXISTS history_command ON history(command);
";

/// The machine and account this program is running as.
pub struct Host {
    pub hostname: String,
    pub username: String,
}

/// One row of the host list, with its command count.
pub struct HostSummary {
    pub id: i64,
    pub hostname: String,
    pub username: String,
    pub commands: i64,
    pub last_import: Option<String>,
}

/// Current UTC time as an ISO 8601 string, e.g. 2026-11-06T18:31:00Z.
pub fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// history.sqlite3 in the same folder as the running executable.
pub fn default_database() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or("could not determine the executable's folder")?;
    Ok(dir.join("history.sqlite3"))
}

/// Open (or create) the database and make sure the schema exists.
pub fn open(path: &Path) -> Result<Connection> {
    if path.is_file() {
        let mut header = [0u8; 8];
        let read = File::open(path)?.read(&mut header)?;
        if &header[..read] == ENCRYPTED_MAGIC {
            return Err(format!(
                "{} was written by the encrypted Python version of PS-HIST; \
                 decrypt it with that version first or use --db with a new file",
                path.display()
            )
            .into());
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let conn = Connection::open(path)?;
    // Rollback journal instead of WAL: no -wal/-shm side files left on a pulled flash drive.
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA foreign_keys = ON;")?;

    // Refuse a database created by the Python version (no host_id column).
    let has_history: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'history'",
        [],
        |row| row.get(0),
    )?;
    if has_history > 0 {
        let has_host_id: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('history') WHERE name = 'host_id'",
            [],
            |row| row.get(0),
        )?;
        if has_host_id == 0 {
            return Err(format!(
                "{} uses the old single-host format; use --db with a new file",
                path.display()
            )
            .into());
        }
    }

    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Identify the machine and user this program is running as.
pub fn current_host() -> Host {
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let username = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    Host { hostname, username }
}

/// Add the host if it is new, and return its id either way.
pub fn register_host(conn: &Connection, host: &Host) -> Result<i64> {
    conn.execute(
        "INSERT INTO hosts(hostname, username, first_seen) VALUES (?1, ?2, ?3)
         ON CONFLICT(hostname, username) DO NOTHING",
        params![host.hostname, host.username, now_stamp()],
    )?;
    let id = conn.query_row(
        "SELECT id FROM hosts WHERE hostname = ?1 AND username = ?2",
        params![host.hostname, host.username],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Every known host with its command count, sorted by name.
pub fn list_hosts(conn: &Connection) -> Result<Vec<HostSummary>> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.hostname, h.username, COUNT(c.id), h.last_import
         FROM hosts h
         LEFT JOIN history c ON c.host_id = h.id
         GROUP BY h.id
         ORDER BY h.hostname, h.username",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(HostSummary {
            id: row.get(0)?,
            hostname: row.get(1)?,
            username: row.get(2)?,
            commands: row.get(3)?,
            last_import: row.get(4)?,
        })
    })?;
    let mut hosts = Vec::new();
    for row in rows {
        hosts.push(row?);
    }
    Ok(hosts)
}