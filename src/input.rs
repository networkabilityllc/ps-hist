//! Keyboard input, translated from crossterm events into simple key names.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    WordLeft,
    WordRight,
    CtrlC,
    Char(char),
}

/// What arrived while waiting for input.
pub enum Input {
    Key(Key),
    Resize,
    Timeout,
}

/// Wait up to `timeout` for a key press or a terminal resize.
pub fn read(timeout: Duration) -> io::Result<Input> {
    if !event::poll(timeout)? {
        return Ok(Input::Timeout);
    }
    match event::read()? {
        // Windows reports presses, repeats and releases; ignore releases.
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            Ok(translate(key).map(Input::Key).unwrap_or(Input::Timeout))
        }
        Event::Resize(_, _) => Ok(Input::Resize),
        _ => Ok(Input::Timeout),
    }
}

fn translate(key: KeyEvent) -> Option<Key> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let translated = match key.code {
        KeyCode::Left if ctrl => Key::WordLeft,
        KeyCode::Right if ctrl => Key::WordRight,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Char('c') | KeyCode::Char('C') if ctrl && !alt => Key::CtrlC,
        KeyCode::Char(ch) if !ctrl && !alt => Key::Char(ch),
        // AltGr on non-US keyboards arrives as Ctrl+Alt plus the character.
        KeyCode::Char(ch) if ctrl && alt => Key::Char(ch),
        _ => return None,
    };
    Some(translated)
}