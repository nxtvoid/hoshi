//! The language the app's text shows in, after the user's choice and the system's.

use crate::m;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use hoshi_core::Language;

/// The choices of language, in the order they are offered.
pub const LANGUAGES: [Language; 3] = [Language::System, Language::English, Language::Spanish];

pub fn language_label(language: Language) -> String {
    match language {
        Language::System => m::language_system(),
        Language::English => m::language_english(),
        Language::Spanish => m::language_spanish(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    English,
    Spanish,
}

impl Locale {
    /// The locale of a BCP 47 tag such as `es-MX`.
    fn of_tag(tag: &str) -> Self {
        let language = tag.split(['-', '_']).next().unwrap_or_default();
        if language.eq_ignore_ascii_case("es") {
            Self::Spanish
        } else {
            Self::English
        }
    }

    fn of(language: Language, system: Option<&str>) -> Self {
        match language {
            Language::System => system.map_or(Self::English, Self::of_tag),
            Language::English => Self::English,
            Language::Spanish => Self::Spanish,
        }
    }
}

#[cfg(not(test))]
mod shown {
    use super::Locale;
    use std::sync::atomic::{AtomicBool, Ordering};

    static SPANISH: AtomicBool = AtomicBool::new(false);

    pub fn get() -> Locale {
        if SPANISH.load(Ordering::Relaxed) {
            Locale::Spanish
        } else {
            Locale::English
        }
    }

    pub fn set(locale: Locale) {
        SPANISH.store(locale == Locale::Spanish, Ordering::Relaxed);
    }
}

/// Each test has its own, as tests share a process.
#[cfg(test)]
mod shown {
    use super::Locale;
    use std::cell::Cell;

    thread_local! {
        static LOCALE: Cell<Locale> = const { Cell::new(Locale::English) };
    }

    pub fn get() -> Locale {
        LOCALE.get()
    }

    pub fn set(locale: Locale) {
        LOCALE.set(locale);
    }
}

/// The locale messages show in now.
pub fn current() -> Locale {
    shown::get()
}

/// Set again whenever the locale changes, for text that isn't written on every render.
#[derive(Default)]
pub struct LocaleChanged;

impl Global for LocaleChanged {}

/// Shows the app in `language` from now on.
pub fn show(language: Language, cx: &mut App) {
    let locale = Locale::of(language, sys_locale::get_locale().as_deref());
    if locale == current() {
        return;
    }
    shown::set(locale);
    cx.set_global(LocaleChanged);
    cx.refresh_windows();
}

/// A text field whose placeholder follows the locale.
pub fn input(
    placeholder: fn() -> String,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> InputState {
    cx.observe_global_in::<LocaleChanged>(window, move |state, window, cx| {
        state.set_placeholder(placeholder(), window, cx)
    })
    .detach();
    InputState::new(window, cx).placeholder(placeholder())
}

/// A text area whose placeholder follows the locale.
pub fn textarea(
    placeholder: fn() -> String,
    window: &mut Window,
    cx: &mut Context<TextareaState>,
) -> TextareaState {
    cx.observe_global_in::<LocaleChanged>(window, move |state, window, cx| {
        state.set_placeholder(placeholder(), window, cx)
    })
    .detach();
    TextareaState::new(window, cx).placeholder(placeholder())
}

#[cfg(test)]
mod tests {
    use super::{Locale, input, show};
    use crate::m;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
    use hoshi_core::Language;

    #[test]
    fn the_system_language_picks_the_locale_until_one_is_chosen() {
        assert_eq!(Locale::of(Language::System, Some("es-MX")), Locale::Spanish);
        assert_eq!(Locale::of(Language::System, Some("es")), Locale::Spanish);
        assert_eq!(
            Locale::of(Language::System, Some("es_ES.UTF-8")),
            Locale::Spanish
        );
        assert_eq!(
            Locale::of(Language::System, Some("est-EE")),
            Locale::English
        );
        assert_eq!(Locale::of(Language::System, Some("fr")), Locale::English);
        assert_eq!(Locale::of(Language::System, None), Locale::English);
        assert_eq!(
            Locale::of(Language::English, Some("es-MX")),
            Locale::English
        );
        assert_eq!(Locale::of(Language::Spanish, Some("en")), Locale::Spanish);
    }

    #[gpui_kit::test]
    fn changing_the_language_changes_messages_and_placeholders(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut field = None;
        let window = cx.add_empty_window();
        window.update(|window, cx| {
            field = Some(cx.new(|cx| input(m::search_threads, window, cx)));
        });
        let field = field.unwrap();
        let english = m::search_threads();
        let placeholder = |window: &mut VisualTestContext| {
            window.read(|cx| field.read(cx).presentation().placeholder().to_string())
        };

        window.update(|_, cx| show(Language::Spanish, cx));
        window.run_until_parked();
        let spanish = m::search_threads();
        assert_ne!(spanish, english);
        assert_eq!(m::snooze_in_days(1), "En 1 día");
        assert_eq!(m::snooze_in_days(2), "En 2 días");
        assert_eq!(placeholder(window), spanish);

        window.update(|_, cx| show(Language::English, cx));
        window.run_until_parked();
        assert_eq!(placeholder(window), english);
    }
}
