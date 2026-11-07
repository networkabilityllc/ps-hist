//! Finding PowerShell, launching commands, and the clipboard.

use std::path::{Path, PathBuf};

/// pwsh if it is on PATH, otherwise Windows PowerShell.
pub fn find_powershell() -> Option<PathBuf> {
    find_on_path("pwsh").or_else(|| find_on_path("powershell"))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE".to_string())
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(|ext| ext.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &extensions {
            let candidate = dir.join(format!("{}{}", name, ext));
            // symlink_metadata rather than is_file(): Store-installed apps such as
            // wt.exe and pwsh.exe are app-execution-alias reparse points that
            // is_file() cannot always follow.
            let found = std::fs::symlink_metadata(&candidate)
                .map(|meta| !meta.is_dir())
                .unwrap_or(false);
            if found {
                return Some(candidate);
            }
        }
    }
    None
}

/// True when running in the classic console host (conhost) with its own window.
///
/// The classic console host's window class is "ConsoleWindowClass". Windows
/// Terminal, VS Code and other modern hosts give the console a placeholder
/// window of class "PseudoConsoleWindow" instead.
#[cfg(windows)]
fn in_classic_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

    // SAFETY: GetConsoleWindow only queries state; GetClassNameW writes at most
    // buffer.len() UTF-16 units into the buffer we own.
    unsafe {
        let window = GetConsoleWindow();
        if window.is_null() {
            return false;
        }
        let mut buffer = [0u16; 64];
        let len = GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        if len <= 0 {
            return false;
        }
        String::from_utf16_lossy(&buffer[..len as usize]) == "ConsoleWindowClass"
    }
}

/// Open the command in a new PowerShell session (same user and elevation)
/// that runs it and stays open, in the same kind of host PS-HIST is running in.
#[cfg(windows)]
pub fn launch_window(shell: &Path, command: &str) -> std::io::Result<()> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let quoted = format!("'{}'", command.replace('\'', "''"));
    let title_text: String = crate::highlight::display_command(command)
        .chars()
        .take(60)
        .collect();
    let title = format!("'PS-HIST: {}'", title_text.replace('\'', "''"));
    let script = format!(
        "$Host.UI.RawUI.WindowTitle = {}\nWrite-Host 'PS> ' -NoNewline -ForegroundColor Yellow; Write-Host {}\n{}\n",
        title, quoted, command
    );

    // -EncodedCommand takes base64 of UTF-16LE text, which sidesteps every
    // quoting problem between Rust, Windows and PowerShell.
    let bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let encoded = STANDARD.encode(bytes);
    let shell_args = ["-NoLogo", "-NoExit", "-EncodedCommand", encoded.as_str()];

    if in_classic_console() {
        // Name conhost.exe explicitly so the default-terminal setting
        // doesn't hand the window over to Windows Terminal.
        Command::new("cmd")
            .args(["/c", "start", "", "conhost.exe"])
            .arg(shell)
            .args(shell_args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        return Ok(());
    }

        // Windows Terminal sets WT_SESSION in every session it hosts. VS Code
    // terminals can inherit it when VS Code was started from Windows Terminal,
    // so skip those (VS Code sets TERM_PROGRAM=vscode).
    let in_vscode = std::env::var("TERM_PROGRAM").map(|value| value == "vscode").unwrap_or(false);
    let wt = if in_vscode {
        None
    } else {
        std::env::var_os("WT_SESSION").and_then(|_| find_on_path("wt"))
    };
    if let Some(wt) = wt {
        // -w 0: the most recently used Windows Terminal window. Use "new" instead
        // of "0" to open a separate window every time.
        Command::new(wt)
            .args(["-w", "0", "new-tab"])
            .arg(shell)
            .args(shell_args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    } else {
        // "start" gives the new console normal window placement. The empty
        // string is start's window-title argument.
        Command::new("cmd")
            .args(["/c", "start", ""])
            .arg(shell)
            .args(shell_args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    }
    Ok(())
}

/// Run the command in this terminal and return its exit code.
#[cfg(not(windows))]
pub fn run_inline(shell: &Path, command: &str) -> std::io::Result<i32> {
    let status = std::process::Command::new(shell)
        .args(["-NoLogo", "-NoProfile", "-Command", command])
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

pub fn copy_to_clipboard(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => clipboard.set_text(text).is_ok(),
        Err(_) => false,
    }
}