//! What a key sends to the shell, as xterm encodes it. Printable text is left to the
//! input handler, so layouts, AltGr and dead keys compose as the system does.

use gpui_kit::Keystroke;

/// Keys the app handles even while a terminal has the keyboard.
pub fn is_app_shortcut(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    modifiers.control && !modifiers.alt && !modifiers.shift && keystroke.key == "j"
}

/// `None` when the key types text, belongs to the app, or means nothing to a terminal.
pub fn key_bytes(keystroke: &Keystroke, app_cursor: bool) -> Option<String> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform {
        return None;
    }
    let code = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = |end: char| {
        if code > 1 {
            format!("\x1b[1;{code}{end}")
        } else if app_cursor {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let function = |end: char| {
        if code > 1 {
            format!("\x1b[1;{code}{end}")
        } else {
            format!("\x1bO{end}")
        }
    };
    let tilde = |number: u8| {
        if code > 1 {
            format!("\x1b[{number};{code}~")
        } else {
            format!("\x1b[{number}~")
        }
    };
    let bytes = match keystroke.key.as_str() {
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" if modifiers.alt => "\x1b\r".into(),
        "enter" => "\r".into(),
        "tab" if modifiers.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "backspace" if modifiers.control => "\x08".into(),
        "backspace" if modifiers.alt => "\x1b\x7f".into(),
        "backspace" => "\x7f".into(),
        "escape" => "\x1b".into(),
        "space" if modifiers.control => "\0".into(),
        "space" if modifiers.alt && !cfg!(target_os = "macos") => "\x1b ".into(),
        key => return character(keystroke, key),
    };
    Some(bytes)
}

fn character(keystroke: &Keystroke, key: &str) -> Option<String> {
    let modifiers = &keystroke.modifiers;
    let mut chars = key.chars();
    let (Some(base), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if modifiers.control && modifiers.alt {
        return None;
    }
    if modifiers.control {
        return control_code(base).map(String::from);
    }
    if modifiers.alt && !cfg!(target_os = "macos") {
        let typed = keystroke
            .key_char
            .clone()
            .unwrap_or_else(|| base.to_string());
        return Some(format!("\x1b{typed}"));
    }
    None
}

fn control_code(key: char) -> Option<char> {
    let code = match key.to_ascii_lowercase() {
        letter @ 'a'..='z' => letter as u8 - b'a' + 1,
        '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    Some(code as char)
}

/// Text pasted into the shell: line breaks as Enter, wrapped when the shell asked to tell
/// pastes from typing.
pub fn paste_bytes(text: &str, bracketed: bool) -> String {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{is_app_shortcut, key_bytes, paste_bytes};
    use gpui_kit::Keystroke;

    fn bytes(source: &'static str) -> Option<String> {
        key_bytes(&Keystroke::parse(source).unwrap(), false)
    }

    #[test]
    fn editing_keys_send_their_control_characters() {
        assert_eq!(bytes("enter").as_deref(), Some("\r"));
        assert_eq!(bytes("backspace").as_deref(), Some("\x7f"));
        assert_eq!(bytes("ctrl-backspace").as_deref(), Some("\x08"));
        assert_eq!(bytes("tab").as_deref(), Some("\t"));
        assert_eq!(bytes("shift-tab").as_deref(), Some("\x1b[Z"));
        assert_eq!(bytes("escape").as_deref(), Some("\x1b"));
    }

    #[test]
    fn ctrl_letters_send_c0_codes() {
        assert_eq!(bytes("ctrl-c").as_deref(), Some("\x03"));
        assert_eq!(bytes("ctrl-d").as_deref(), Some("\x04"));
        assert_eq!(bytes("ctrl-a").as_deref(), Some("\x01"));
        assert_eq!(bytes("ctrl-z").as_deref(), Some("\x1a"));
        assert_eq!(bytes("ctrl-[").as_deref(), Some("\x1b"));
        assert_eq!(bytes("ctrl-space").as_deref(), Some("\0"));
    }

    #[test]
    fn arrows_follow_the_cursor_mode_and_modifiers() {
        assert_eq!(bytes("up").as_deref(), Some("\x1b[A"));
        assert_eq!(bytes("left").as_deref(), Some("\x1b[D"));
        let up = Keystroke::parse("up").unwrap();
        assert_eq!(key_bytes(&up, true).as_deref(), Some("\x1bOA"));
        assert_eq!(bytes("ctrl-right").as_deref(), Some("\x1b[1;5C"));
        assert_eq!(bytes("shift-up").as_deref(), Some("\x1b[1;2A"));
        assert_eq!(bytes("delete").as_deref(), Some("\x1b[3~"));
        assert_eq!(bytes("ctrl-delete").as_deref(), Some("\x1b[3;5~"));
        assert_eq!(bytes("f1").as_deref(), Some("\x1bOP"));
        assert_eq!(bytes("f5").as_deref(), Some("\x1b[15~"));
    }

    #[test]
    fn typed_text_is_left_to_the_input_handler() {
        assert_eq!(bytes("a"), None);
        assert_eq!(bytes("shift-a"), None);
        assert_eq!(bytes("space"), None);
        assert_eq!(bytes("ctrl-alt-2"), None);
        assert_eq!(bytes("cmd-c"), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(bytes("alt-b").as_deref(), Some("\x1bb"));
        assert_eq!(bytes("alt-backspace").as_deref(), Some("\x1b\x7f"));
    }

    #[test]
    fn ctrl_j_belongs_to_the_app() {
        assert!(is_app_shortcut(&Keystroke::parse("ctrl-j").unwrap()));
        assert!(!is_app_shortcut(&Keystroke::parse("ctrl-k").unwrap()));
        assert!(!is_app_shortcut(&Keystroke::parse("j").unwrap()));
    }

    #[test]
    fn pastes_send_lines_as_enter_and_bracket_when_asked() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), "a\rb\rc");
        assert_eq!(paste_bytes("ls", true), "\x1b[200~ls\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), "\x1b[200~xy\x1b[201~");
    }
}
