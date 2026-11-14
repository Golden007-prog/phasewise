//! Pieces both screens share: notices, the spinner, the two-row confirmation
//! pickers, text editing and footer items. Pure: no terminal, no I/O.

use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// How serious a message is; the renderer colours it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Error,
    Info,
}

/// The transient result of the last action, shown above the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

impl Notice {
    fn new(level: Level, text: impl Into<String>) -> Notice {
        Notice {
            level,
            text: text.into(),
        }
    }

    pub fn ok(text: impl Into<String>) -> Notice {
        Notice::new(Level::Ok, text)
    }

    pub fn warn(text: impl Into<String>) -> Notice {
        Notice::new(Level::Warn, text)
    }

    pub fn error(text: impl Into<String>) -> Notice {
        Notice::new(Level::Error, text)
    }

    pub fn info(text: impl Into<String>) -> Notice {
        Notice::new(Level::Info, text)
    }
}

/// Spinner frames of the running work.
pub const SPINNER: [&str; 10] = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
/// How long one spinner frame lasts.
pub const FRAME_TIME: Duration = Duration::from_millis(120);

/// Index of the spinner frame for the time elapsed since a fixed start.
pub fn spinner_frame(elapsed: Duration) -> usize {
    let frames = elapsed.as_millis() / FRAME_TIME.as_millis();
    usize::try_from(frames % SPINNER.len() as u128).unwrap_or(0)
}

/// The glyph of a frame index (any index; it wraps).
pub fn spinner(frame: usize) -> &'static str {
    SPINNER.get(frame % SPINNER.len()).copied().unwrap_or("·")
}

/// What a key means in a two-row confirmation (row 0 acts, row 1 cancels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Act,
    Cancel,
    /// Stay open with this row highlighted.
    Row(usize),
}

pub fn confirm_key(key: KeyEvent, row: usize) -> Choice {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Choice::Row(row);
    }
    match key.code {
        KeyCode::Char('y' | 'Y' | '1') => Choice::Act,
        KeyCode::Char('n' | 'N' | '2') | KeyCode::Esc => Choice::Cancel,
        KeyCode::Char('j') | KeyCode::Down => Choice::Row(1),
        KeyCode::Char('k') | KeyCode::Up => Choice::Row(0),
        KeyCode::Enter if row == 0 => Choice::Act,
        KeyCode::Enter => Choice::Cancel,
        _ => Choice::Row(row),
    }
}

/// The row a digit key (1 to 9) picks directly in a picker.
pub fn digit_row(key: KeyEvent) -> Option<usize> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char(c @ '1'..='9') => Some(usize::from(c as u8 - b'1')),
        _ => None,
    }
}

/// Applies a text-editing key to `buffer`: characters, Backspace, Ctrl-U
/// (clear) and Ctrl-W (delete the last word). False for any other key.
pub fn edit_text(buffer: &mut String, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => {
            buffer.pop();
            true
        }
        KeyCode::Char('u') if ctrl => {
            buffer.clear();
            true
        }
        KeyCode::Char('w') if ctrl => {
            let kept = buffer.trim_end().len();
            buffer.truncate(kept);
            let start = buffer
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map_or(0, |(i, c)| i + c.len_utf8());
            buffer.truncate(start);
            true
        }
        KeyCode::Char(c) if !ctrl => {
            buffer.push(c);
            true
        }
        _ => false,
    }
}

/// A footer entry: key and action.
pub type FooterItem = (String, String);

pub fn item(key: &str, action: &str) -> FooterItem {
    (key.to_string(), action.to_string())
}

/// Footer of a picker with `rows` rows.
pub fn picker_footer(rows: usize) -> Vec<FooterItem> {
    vec![
        item("enter", "choose"),
        item(&format!("1–{}", rows.clamp(1, 9)), "jump"),
        item("esc", "cancel"),
    ]
}

/// Shortcuts of a screen, in groups: `(group, [(key, description)])`.
pub type ShortcutGroups = Vec<(&'static str, Vec<(&'static str, &'static str)>)>;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn spinner_advances_every_120_ms_and_wraps() {
        assert_eq!(spinner_frame(Duration::from_millis(0)), 0);
        assert_eq!(spinner_frame(Duration::from_millis(119)), 0);
        assert_eq!(spinner_frame(Duration::from_millis(120)), 1);
        assert_eq!(spinner_frame(Duration::from_millis(1200)), 0);
        assert_eq!(spinner(4), "✻");
        assert_eq!(spinner(14), "✻");
    }

    #[test]
    fn confirmation_keys() {
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(confirm_key(key('y'), 1), Choice::Act);
        assert_eq!(confirm_key(key('1'), 1), Choice::Act);
        assert_eq!(confirm_key(key('n'), 0), Choice::Cancel);
        assert_eq!(confirm_key(key('2'), 0), Choice::Cancel);
        assert_eq!(confirm_key(esc, 0), Choice::Cancel);
        assert_eq!(confirm_key(key('j'), 0), Choice::Row(1));
        assert_eq!(confirm_key(key('k'), 1), Choice::Row(0));
        assert_eq!(confirm_key(enter, 0), Choice::Act);
        assert_eq!(confirm_key(enter, 1), Choice::Cancel);
        assert_eq!(confirm_key(key('z'), 1), Choice::Row(1));
        assert_eq!(confirm_key(ctrl('y'), 0), Choice::Row(0));
    }

    #[test]
    fn digits_pick_rows_one_to_nine() {
        assert_eq!(digit_row(key('1')), Some(0));
        assert_eq!(digit_row(key('9')), Some(8));
        assert_eq!(digit_row(key('0')), None);
        assert_eq!(digit_row(key('a')), None);
        assert_eq!(digit_row(ctrl('1')), None);
    }

    #[test]
    fn text_editing_keys() {
        let mut s = String::from("add a CLI");
        assert!(edit_text(&mut s, ctrl('w')));
        assert_eq!(s, "add a ");
        assert!(edit_text(&mut s, ctrl('w')));
        assert_eq!(s, "add ");
        assert!(edit_text(&mut s, key('x')));
        assert!(edit_text(
            &mut s,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
        ));
        assert_eq!(s, "add ");
        assert!(edit_text(&mut s, ctrl('u')));
        assert_eq!(s, "");
        assert!(edit_text(&mut s, ctrl('w')));
        assert!(!edit_text(&mut s, ctrl('a')));
        assert!(!edit_text(
            &mut s,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ));
    }

    #[test]
    fn footer_of_a_picker_names_its_rows() {
        assert_eq!(picker_footer(2)[1], item("1–2", "jump"));
        assert_eq!(picker_footer(30)[1], item("1–9", "jump"));
    }
}
