mod app;
mod db;
mod import;
mod input;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Parser;

/// Browse, search, and run commands from PSReadLine history.
#[derive(Parser)]
#[command(name = "ps-hist", version)]
struct Args {
    /// SQLite database path (default: history.sqlite3 beside the executable)
    #[arg(long)]
    db: Option<PathBuf>,

    /// Additional history file to import for this host (repeatable)
    #[arg(long)]
    history_file: Vec<PathBuf>,

    /// Import history and print a report without opening the interactive UI
    #[arg(long)]
    import_only: bool,
}

fn main() -> db::Result<()> {
    let args = Args::parse();
    let import_only = args.import_only;
    let db_path = match args.db {
        Some(path) => path,
        None => db::default_database()?,
    };

    let mut conn = db::open(&db_path)?;
    let host = db::current_host();
    let host_id = db::register_host(&conn, &host)?;
    let host_label = format!("{}\\{}", host.hostname, host.username);

    println!("Database:  {}", db_path.display());
    println!("This host: {} (id {})", host_label, host_id);
    println!();

    let mut files = import::default_history_files();
    for extra in args.history_file {
        if !files.contains(&extra) {
            files.push(extra);
        }
    }

    let mut total = 0;
    if files.is_empty() {
        println!("No history files found. Use --history-file to specify one.");
    } else {
        println!("Import:");
        for path in &files {
            match import::import_file(&mut conn, host_id, path) {
                Ok(report) => {
                    total += report.added;
                    println!(
                        "  {:<9} {:>7} new of {:>7} read  {}",
                        report.mode.label(),
                        report.added,
                        report.entries,
                        report.path.display()
                    );
                }
                Err(error) => println!("  error     {}: {}", path.display(), error),
            }
        }
        println!("  {} new commands in total", total);
        import::mark_imported(&conn, host_id)?;
    }

    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if import_only || !interactive {
        println!();
        println!("Known hosts:");
        for h in db::list_hosts(&conn)? {
            let last = h.last_import.as_deref().unwrap_or("never");
            println!(
                "  [{}] {}\\{}  {} commands  last import: {}",
                h.id, h.hostname, h.username, h.commands, last
            );
        }
        return Ok(());
    }

    let status = format!("{} new entries imported", ui::thousands(total as i64));
    app::App::new(conn, db_path, host_id, host_label, files, status).run()
}