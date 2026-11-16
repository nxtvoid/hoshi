//! Mouse events reported to a full-screen program (vim, htop) that asked for them, encoded
//! as xterm does.

use alacritty_terminal::term::TermMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    /// Moving with no button held.
    None,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// Whether the program wants the mouse; Shift keeps it for selecting text.
pub fn reporting(mode: TermMode, shift: bool) -> bool {
    mode.intersects(TermMode::MOUSE_MODE) && !shift
}

/// What to send the program, `None` when its mode doesn't report this event. `line` and
/// `column` are the 0-based cell on screen.
pub fn report(
    mode: TermMode,
    button: Button,
    action: Action,
    line: usize,
    column: usize,
    modifiers: Modifiers,
) -> Option<String> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    let wheel = matches!(button, Button::WheelUp | Button::WheelDown);
    match action {
        Action::Motion if mode.contains(TermMode::MOUSE_MOTION) => {}
        Action::Motion if mode.contains(TermMode::MOUSE_DRAG) && button != Button::None => {}
        Action::Motion => return None,
        Action::Release if wheel => return None,
        Action::Press if button == Button::None => return None,
        _ => {}
    }
    let base: u32 = match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::None => 3,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
    };
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let mut code = if action == Action::Release && !sgr {
        3
    } else {
        base
    };
    if action == Action::Motion {
        code += 32;
    }
    code += 4 * u32::from(modifiers.shift)
        + 8 * u32::from(modifiers.alt)
        + 16 * u32::from(modifiers.control);
    let (x, y) = (column as u32 + 1, line as u32 + 1);
    if sgr {
        let end = if action == Action::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{end}"));
    }
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    let encode = |value: u32| {
        let value = 32 + value;
        let limit = if utf8 { 2047 } else { 127 };
        (value <= limit).then(|| char::from_u32(value)).flatten()
    };
    Some(format!(
        "\x1b[M{}{}{}",
        encode(code)?,
        encode(x)?,
        encode(y)?
    ))
}

#[cfg(test)]
mod tests {
    use super::{Action, Button, Modifiers, report, reporting};
    use alacritty_terminal::term::TermMode;

    const CLICKS: TermMode = TermMode::MOUSE_REPORT_CLICK;

    #[test]
    fn nothing_is_reported_unless_the_program_asked() {
        let none = Modifiers::default();
        assert_eq!(
            report(TermMode::empty(), Button::Left, Action::Press, 0, 0, none),
            None
        );
        assert!(!reporting(CLICKS, true));
        assert!(reporting(CLICKS, false));
    }

    #[test]
    fn sgr_reports_press_and_release_with_the_button() {
        let mode = CLICKS | TermMode::SGR_MOUSE;
        let none = Modifiers::default();
        assert_eq!(
            report(mode, Button::Left, Action::Press, 4, 9, none).as_deref(),
            Some("\x1b[<0;10;5M")
        );
        assert_eq!(
            report(mode, Button::Right, Action::Release, 0, 0, none).as_deref(),
            Some("\x1b[<2;1;1m")
        );
        let control = Modifiers {
            control: true,
            ..none
        };
        assert_eq!(
            report(mode, Button::WheelDown, Action::Press, 1, 1, control).as_deref(),
            Some("\x1b[<81;2;2M")
        );
        assert_eq!(
            report(mode, Button::WheelUp, Action::Release, 1, 1, none),
            None
        );
    }

    #[test]
    fn motion_follows_the_mode() {
        let none = Modifiers::default();
        let clicks = CLICKS | TermMode::SGR_MOUSE;
        assert_eq!(
            report(clicks, Button::Left, Action::Motion, 0, 0, none),
            None
        );
        let drag = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        assert_eq!(
            report(drag, Button::Left, Action::Motion, 0, 2, none).as_deref(),
            Some("\x1b[<32;3;1M")
        );
        assert_eq!(report(drag, Button::None, Action::Motion, 0, 2, none), None);
        let motion = TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE;
        assert_eq!(
            report(motion, Button::None, Action::Motion, 0, 2, none).as_deref(),
            Some("\x1b[<35;3;1M")
        );
    }

    #[test]
    fn x10_encodes_values_as_characters_and_releases_as_button_three() {
        let none = Modifiers::default();
        assert_eq!(
            report(CLICKS, Button::Left, Action::Press, 0, 0, none).as_deref(),
            Some("\x1b[M !!")
        );
        assert_eq!(
            report(CLICKS, Button::Left, Action::Release, 2, 1, none).as_deref(),
            Some("\x1b[M#\"#")
        );
        assert_eq!(
            report(CLICKS, Button::Left, Action::Press, 0, 200, none),
            None
        );
        let utf8 = CLICKS | TermMode::UTF8_MOUSE;
        assert_eq!(
            report(utf8, Button::Left, Action::Press, 0, 200, none),
            Some(format!("\x1b[M {}!", char::from_u32(233).unwrap()))
        );
    }
}
