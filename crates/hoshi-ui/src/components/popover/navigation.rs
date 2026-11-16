//! Which item of a menu the keyboard is on: arrows, Home and End over the enabled items, and
//! typing the start of a label to jump to it.

use std::time::{Duration, Instant};

/// How long typed characters keep adding up before typing starts over.
const TYPEAHEAD_RESET: Duration = Duration::from_millis(500);

/// One item as navigation sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavItem {
    pub label: String,
    pub enabled: bool,
}

/// The highlighted item and what has been typed.
#[derive(Clone, Debug, Default)]
pub struct Navigation {
    pub highlighted: Option<usize>,
    typed: String,
    typed_at: Option<Instant>,
}

impl Navigation {
    pub fn first(&mut self, items: &[NavItem]) -> Option<usize> {
        self.highlighted = items.iter().position(|item| item.enabled);
        self.highlighted
    }

    pub fn last(&mut self, items: &[NavItem]) -> Option<usize> {
        self.highlighted = items.iter().rposition(|item| item.enabled);
        self.highlighted
    }

    /// The next enabled item after the highlighted one, or the first with none; wraps when
    /// `wrap` is set.
    pub fn next(&mut self, items: &[NavItem], wrap: bool) -> Option<usize> {
        let Some(from) = self.highlighted.filter(|_| !items.is_empty()) else {
            return self.first(items);
        };
        let after = (from + 1..items.len()).find(|&index| items[index].enabled);
        let found = after.or_else(|| {
            wrap.then(|| (0..=from.min(items.len() - 1)).find(|&index| items[index].enabled))
                .flatten()
        });
        if found.is_some() {
            self.highlighted = found;
        }
        self.highlighted
    }

    /// The previous enabled item, or the last with none; wraps when `wrap` is set.
    pub fn previous(&mut self, items: &[NavItem], wrap: bool) -> Option<usize> {
        let Some(from) = self.highlighted.filter(|_| !items.is_empty()) else {
            return self.last(items);
        };
        let before = (0..from.min(items.len()))
            .rev()
            .find(|&index| items[index].enabled);
        let found = before.or_else(|| {
            wrap.then(|| {
                (from..items.len())
                    .rev()
                    .find(|&index| items[index].enabled)
            })
            .flatten()
        });
        if found.is_some() {
            self.highlighted = found;
        }
        self.highlighted
    }

    /// Whether a typed space adds to the search instead of choosing the item.
    pub fn is_typing(&self, now: Instant) -> bool {
        !self.typed.is_empty()
            && self
                .typed_at
                .is_some_and(|at| now.duration_since(at) < TYPEAHEAD_RESET)
    }

    /// Adds `character` to what was typed and highlights the first enabled item whose label
    /// starts with it, searching from the highlighted one. The same letter typed again moves
    /// on to the next item starting with it.
    pub fn type_char(&mut self, items: &[NavItem], character: char, now: Instant) -> Option<usize> {
        if !self.is_typing(now) {
            self.typed.clear();
        }
        self.typed.extend(character.to_lowercase());
        self.typed_at = Some(now);

        let mut chars = self.typed.chars();
        let first = chars.next()?;
        let repeated = chars.all(|other| other == first);
        let (needle, skip_current) = if repeated {
            (first.to_string(), true)
        } else {
            (self.typed.clone(), false)
        };
        let count = items.len();
        if count == 0 {
            return None;
        }
        let start = match (self.highlighted, skip_current) {
            (Some(index), true) => index + 1,
            (Some(index), false) => index,
            (None, _) => 0,
        };
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| {
                let item = &items[index];
                item.enabled && item.label.to_lowercase().starts_with(&needle)
            });
        if found.is_some() {
            self.highlighted = found;
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[(&str, bool)]) -> Vec<NavItem> {
        labels
            .iter()
            .map(|(label, enabled)| NavItem {
                label: (*label).into(),
                enabled: *enabled,
            })
            .collect()
    }

    #[test]
    fn arrows_skip_disabled_items_and_wrap() {
        let list = items(&[
            ("Ask", true),
            ("Edits", false),
            ("Auto", true),
            ("Full", true),
        ]);
        let mut nav = Navigation::default();
        assert_eq!(
            nav.next(&list, true),
            Some(0),
            "the first arrow lands on the first"
        );
        assert_eq!(
            nav.next(&list, true),
            Some(2),
            "the disabled one is skipped"
        );
        assert_eq!(nav.next(&list, true), Some(3));
        assert_eq!(nav.next(&list, true), Some(0), "past the last it wraps");
        assert_eq!(nav.previous(&list, true), Some(3), "and back");
        assert_eq!(nav.previous(&list, false), Some(2));
        nav.highlighted = Some(3);
        assert_eq!(nav.next(&list, false), Some(3), "without wrapping it stays");
    }

    #[test]
    fn up_with_nothing_highlighted_lands_on_the_last_enabled() {
        let list = items(&[("a", true), ("b", true), ("c", false)]);
        let mut nav = Navigation::default();
        assert_eq!(nav.previous(&list, true), Some(1));
        assert_eq!(nav.first(&list), Some(0));
        assert_eq!(nav.last(&list), Some(1));
    }

    #[test]
    fn a_list_of_disabled_items_highlights_nothing() {
        let list = items(&[("a", false), ("b", false)]);
        let mut nav = Navigation::default();
        assert_eq!(nav.next(&list, true), None);
        assert_eq!(nav.previous(&list, true), None);
    }

    #[test]
    fn typing_jumps_to_the_label_that_starts_with_it() {
        let list = items(&[
            ("Sonnet", true),
            ("Opus", true),
            ("Opus Legacy", false),
            ("Haiku", true),
        ]);
        let now = Instant::now();
        let mut nav = Navigation::default();
        assert_eq!(nav.type_char(&list, 'h', now), Some(3));
        assert_eq!(
            nav.type_char(&list, 'x', now),
            None,
            "no match keeps the highlight"
        );
        assert_eq!(nav.highlighted, Some(3));

        let later = now + TYPEAHEAD_RESET;
        assert_eq!(
            nav.type_char(&list, 'O', later),
            Some(1),
            "case does not matter"
        );
        assert_eq!(
            nav.type_char(&list, 'p', later),
            Some(1),
            "more letters narrow from the current"
        );
    }

    #[test]
    fn the_same_letter_again_cycles_and_skips_disabled() {
        let list = items(&[
            ("Auto", true),
            ("Ask", true),
            ("All", false),
            ("Accept", true),
        ]);
        let now = Instant::now();
        let mut nav = Navigation::default();
        assert_eq!(nav.type_char(&list, 'a', now), Some(0));
        assert_eq!(nav.type_char(&list, 'a', now), Some(1));
        assert_eq!(nav.type_char(&list, 'a', now), Some(3));
        assert_eq!(nav.type_char(&list, 'a', now), Some(0));
        assert!(nav.is_typing(now));
        assert!(!nav.is_typing(now + TYPEAHEAD_RESET));
    }
}
