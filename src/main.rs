mod app;
mod db;
mod highlight;
mod import;
mod input;
mod shell;
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

    /// PowerShell executable (defaults to pwsh, then powershell)
    #[arg(long)]
    shell: Option<PathBuf>,

    /// Import history and print a report without opening the interactive UI
    #[arg(long)]
    import_only: bool,
}

fn main() -> db::Result<()> {
    let args = Args::parse();
    let import_only = args.import_only;
    let shell = match args.shell {
        Some(path) => path,
        None => shell::find_powershell()
            .ok_or("could not find pwsh or powershell; pass --shell with its path")?,
    };
    let db_path = match args.db {
        Some(path) => path,
        None => db::default_database()?,
    };

    let mut conn = db::open(&db_path)?;
    let host = db::current_host();
    let host_id = db::register_host(&conn, &host)?;
    let host_label = format!("{}\\{}", host.hostname, host.username);

    let mut files = import::default_history_files();
    for extra in args.history_file {
        if !files.contains(&extra) {
            files.push(extra);
        }
    }

    // Import first, collecting the report instead of printing it right away.
    let mut report = vec![
        format!("Database:  {}", db_path.display()),
        format!("This host: {} (id {})", host_label, host_id),
        String::new(),
    ];
    let mut total = 0;
    if files.is_empty() {
        report.push("No history files found. Use --history-file to specify one.".to_string());
    } else {
        report.push("Import:".to_string());
        for path in &files {
            match import::import_file(&mut conn, host_id, path) {
                Ok(result) => {
                    total += result.added;
                    report.push(format!(
                        "  {:<9} {:>7} new of {:>7} read  {}",
                        result.mode.label(),
                        result.added,
                        result.entries,
                        result.path.display()
                    ));
                }
                Err(error) => report.push(format!("  error     {}: {}", path.display(), error)),
            }
        }
        report.push(format!("  {} new commands in total", total));
        import::mark_imported(&conn, host_id)?;
    }

    // Print the report only when not opening the interactive UI.
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if import_only || !interactive {
        for line in &report {
            println!("{}", line);
        }
        println!();
        println!("Known hosts:");
        for h in db::list_hosts(&conn)? {
            let last = h.last_import.as_deref().unwrap_or("never");
            println!(
                "  [{}] {}\\{}  {} commands  last import: {}",
                h.id, h.hostname, h.username, h.commands, last
            );
        }
        if !import_only {
            println!();
            println!("Run this program in a terminal to use the interactive menu.");
        }
        return Ok(());
    }

    let status = format!("{} new entries imported", ui::thousands(total as i64));
    app::App::new(conn, db_path, host_id, host_label, files, shell, status).run()
}