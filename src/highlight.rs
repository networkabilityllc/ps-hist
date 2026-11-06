//! Command display: newline markers, truncation, and PowerShell keyword highlighting.

use std::sync::LazyLock;

use regex::Regex;

use crate::ui::Screen;

/// Cmdlet-style verbs, $variables, and true/false/null.
static KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\b(?:get|set|new|remove|write|read|start|stop|select|where|foreach|if|else|function|param|Import|Export)-[\w-]+\b|\$[\w:]+|\b(?:true|false|null)\b)",
    )
    .expect("keyword pattern is valid")
});

/// Show embedded newlines as ↵ and blank out other control characters,
/// so a multi-line command fits on one row.
pub fn display_command(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\r' || ch == '\n' {
                '↵'
            } else if ch.is_control() {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// Draw a command in exactly `width` columns: truncated with … if too long,
/// padded with spaces if short, with keywords in the accent style.
pub fn draw_command(
    scr: &mut Screen,
    row: i32,
    col: i32,
    width: i32,
    text: &str,
    base: &str,
    accent: &str,
) {
    if width <= 0 {
        return;
    }
    let mut text = display_command(text);
    if text.chars().count() as i32 > width {
        let kept: String = text.chars().take((width - 1) as usize).collect();
        text = kept + "…";
    }

    let mut pos = 0;
    let mut c = col;
    for found in KEYWORD.find_iter(&text) {
        if found.start() > pos {
            let part = &text[pos..found.start()];
            scr.at(row, c, part, base);
            c += part.chars().count() as i32;
        }
        scr.at(row, c, found.as_str(), accent);
        c += found.as_str().chars().count() as i32;
        pos = found.end();
    }
    if pos < text.len() {
        let part = &text[pos..];
        scr.at(row, c, part, base);
        c += part.chars().count() as i32;
    }

    let drawn = c - col;
    if drawn < width {
        scr.at(row, c, &" ".repeat((width - drawn) as usize), base);
    }
}