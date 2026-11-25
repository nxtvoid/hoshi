//! The keys an agent or the user presses, as the browser's input events describe them.

const ALT: u8 = 1;
const CONTROL: u8 = 2;
const META: u8 = 4;
const SHIFT: u8 = 8;

/// The keys of a US keyboard that type a symbol: alone, with Shift, their code and number.
const SYMBOLS: [(char, char, &str, i64); 21] = [
    ('0', ')', "Digit0", 48),
    ('1', '!', "Digit1", 49),
    ('2', '@', "Digit2", 50),
    ('3', '#', "Digit3", 51),
    ('4', '$', "Digit4", 52),
    ('5', '%', "Digit5", 53),
    ('6', '^', "Digit6", 54),
    ('7', '&', "Digit7", 55),
    ('8', '*', "Digit8", 56),
    ('9', '(', "Digit9", 57),
    (';', ':', "Semicolon", 186),
    ('=', '+', "Equal", 187),
    (',', '<', "Comma", 188),
    ('-', '_', "Minus", 189),
    ('.', '>', "Period", 190),
    ('/', '?', "Slash", 191),
    ('`', '~', "Backquote", 192),
    ('[', '{', "BracketLeft", 219),
    ('\\', '|', "Backslash", 220),
    (']', '}', "BracketRight", 221),
    ('\'', '"', "Quote", 222),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Key {
    pub key: String,
    pub code: String,
    pub number: i64,
    /// What it types; `None` for keys that type nothing.
    pub text: Option<String>,
}

/// The key that types `character`, `None` when a US keyboard has none.
pub(super) fn typed(character: char) -> Option<Key> {
    let (code, number) = match character {
        ' ' => ("Space".to_owned(), 32),
        letter if letter.is_ascii_alphabetic() => {
            let upper = letter.to_ascii_uppercase();
            (format!("Key{upper}"), i64::from(u32::from(upper)))
        }
        symbol => SYMBOLS
            .iter()
            .find(|(alone, shifted, ..)| [*alone, *shifted].contains(&symbol))
            .map(|(_, _, code, number)| ((*code).to_owned(), *number))?,
    };
    Some(Key {
        key: character.to_string(),
        code,
        number,
        text: Some(character.to_string()),
    })
}

fn named(name: &str) -> Option<Key> {
    let (key, code, number, text) = match name {
        "Enter" => ("Enter", "Enter", 13, Some("\r")),
        "Tab" => ("Tab", "Tab", 9, None),
        "Escape" => ("Escape", "Escape", 27, None),
        "Backspace" => ("Backspace", "Backspace", 8, None),
        "Delete" => ("Delete", "Delete", 46, None),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37, None),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38, None),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39, None),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40, None),
        "Home" => ("Home", "Home", 36, None),
        "End" => ("End", "End", 35, None),
        "PageUp" => ("PageUp", "PageUp", 33, None),
        "PageDown" => ("PageDown", "PageDown", 34, None),
        "Space" => (" ", "Space", 32, Some(" ")),
        _ => return None,
    };
    Some(Key {
        key: key.into(),
        code: code.into(),
        number,
        text: text.map(str::to_owned),
    })
}

/// A modifier's bit among an event's modifiers, and its key.
fn modifier(name: &str) -> Option<(u8, Key)> {
    let (bit, key, code, number) = match name {
        "Alt" => (ALT, "Alt", "AltLeft", 18),
        "Control" | "Ctrl" | "ControlOrMeta" => (CONTROL, "Control", "ControlLeft", 17),
        "Meta" | "Cmd" | "Command" => (META, "Meta", "MetaLeft", 91),
        "Shift" => (SHIFT, "Shift", "ShiftLeft", 16),
        _ => return None,
    };
    let key = Key {
        key: key.into(),
        code: code.into(),
        number,
        text: None,
    };
    Some((bit, key))
}

/// Keys pressed together.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Chord {
    /// The modifiers held, in order, each with its bit.
    pub held: Vec<(u8, Key)>,
    pub key: Key,
}

/// Reads `Enter`, `a` or `Control+Shift+K`.
pub(super) fn chord(text: &str) -> Option<Chord> {
    let (modifiers, last) = match text.strip_suffix('+') {
        Some("") => ("", "+"),
        Some(rest) => (rest.strip_suffix('+')?, "+"),
        None => text.rsplit_once('+').unwrap_or(("", text)),
    };
    let held = modifiers
        .split('+')
        .filter(|name| !name.is_empty())
        .map(modifier)
        .collect::<Option<Vec<_>>>()?;
    let mut characters = last.chars();
    let single = characters.next().filter(|_| characters.next().is_none());
    let mut key = match single {
        Some(character) => typed(character).unwrap_or(Key {
            key: character.to_string(),
            code: String::new(),
            number: 0,
            text: Some(character.to_string()),
        }),
        None => named(last)?,
    };
    let bits = held.iter().fold(0, |bits, (bit, _)| bits | bit);
    if bits & SHIFT != 0
        && let Some(letter) = single.filter(char::is_ascii_alphabetic)
    {
        key.key = letter.to_ascii_uppercase().to_string();
        key.text = Some(key.key.clone());
    }
    if bits & (ALT | CONTROL | META) != 0 {
        key.text = None;
    }
    Some(Chord { held, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_names_its_modifiers_and_key() {
        let all = chord("Control+a").unwrap();
        assert_eq!(all.held.len(), 1);
        assert_eq!(all.held[0].0, CONTROL);
        assert_eq!((all.key.code.as_str(), all.key.number), ("KeyA", 65));
        assert_eq!(all.key.text, None);

        let shifted = chord("Shift+a").unwrap();
        assert_eq!(shifted.key.text.as_deref(), Some("A"));
        assert_eq!(chord("Control+Shift+K").unwrap().held.len(), 2);
        assert_eq!(chord("Enter").unwrap().key.text.as_deref(), Some("\r"));
        assert_eq!(chord("ñ").unwrap().key.text.as_deref(), Some("ñ"));
        assert_eq!(chord("Hyper+a"), None);
        assert_eq!(chord("Nope"), None);
    }

    #[test]
    fn plus_is_a_key_too() {
        assert_eq!(chord("+").unwrap().key.code, "Equal");
        let zoom = chord("Control++").unwrap();
        assert_eq!((zoom.held.len(), zoom.key.key.as_str()), (1, "+"));
    }

    #[test]
    fn a_symbol_never_passes_for_the_key_its_number_names() {
        assert_eq!(typed('.').unwrap().number, 190);
        assert_eq!(typed('%').unwrap().number, 53);
        assert_eq!(typed('é'), None);
    }
}
