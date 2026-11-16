//! A block's text input: a textarea for prose, which holds chips, or a code editor for a
//! fenced block, which highlights its language. Code editors take no chips.

use gpui_kit::component::input::{EditorState, InlineToken, InlineTokenSpan, TextareaState};
use gpui_kit::*;
use std::ops::Range;

#[derive(Clone)]
pub(super) enum BlockInput {
    Text(Entity<TextareaState>),
    Code(Entity<EditorState>),
}

macro_rules! each {
    ($input:expr, |$state:ident| $body:expr) => {
        match $input {
            BlockInput::Text($state) => $body,
            BlockInput::Code($state) => $body,
        }
    };
}

impl BlockInput {
    pub(super) fn text(window: &mut Window, cx: &mut App) -> Self {
        Self::Text(cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10_000)
                .submit_on_enter(true)
        }))
    }

    pub(super) fn code(language: &str, window: &mut Window, cx: &mut App) -> Self {
        Self::Code(cx.new(|cx| {
            EditorState::new(window, cx)
                .language(SharedString::from(language.to_owned()))
                .line_number(false)
                .folding(false)
                .indent_guides(false)
                .auto_close(false)
                .smart_indent(false)
                .searchable(false)
        }))
    }

    pub(super) fn is_code(&self) -> bool {
        matches!(self, Self::Code(_))
    }

    pub(super) fn as_text(&self) -> Option<&Entity<TextareaState>> {
        match self {
            Self::Text(state) => Some(state),
            Self::Code(_) => None,
        }
    }

    pub(super) fn entity_id(&self) -> EntityId {
        each!(self, |state| state.entity_id())
    }

    pub(super) fn value(&self, cx: &App) -> SharedString {
        each!(self, |state| state.read(cx).value())
    }

    pub(super) fn cursor(&self, cx: &App) -> usize {
        each!(self, |state| state.read(cx).cursor())
    }

    pub(super) fn selected_range(&self, cx: &App) -> Range<usize> {
        each!(self, |state| state.read(cx).selected_range())
    }

    pub(super) fn tokens(&self, cx: &App) -> Vec<InlineTokenSpan> {
        match self {
            Self::Text(state) => state
                .read(cx)
                .tokens()
                .iter()
                .filter(|span| !super::is_mark_token(span.token()))
                .cloned()
                .collect(),
            Self::Code(_) => Vec::new(),
        }
    }

    pub(super) fn is_focused(&self, window: &Window, cx: &App) -> bool {
        each!(self, |state| state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window))
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        each!(self, |state| state
            .update(cx, |state, cx| state.focus(window, cx)))
    }

    pub(super) fn set_selected_range(&self, range: Range<usize>, cx: &mut App) {
        each!(self, |state| state
            .update(cx, |state, cx| state.set_selected_range(range, cx)))
    }

    pub(super) fn set_placeholder(&self, text: SharedString, window: &mut Window, cx: &mut App) {
        each!(self, |state| state.update(cx, |state, cx| state
            .set_placeholder(text, window, cx)))
    }

    /// Where the caret is drawn and the text's bounds, from the last layout.
    pub(super) fn caret_geometry(
        &self,
        cx: &App,
    ) -> Option<(Bounds<Pixels>, Pixels, Bounds<Pixels>)> {
        each!(self, |state| {
            let state = state.read(cx);
            let (caret, line_height) = state.cursor_layout()?;
            Some((caret, line_height, state.text_bounds()?))
        })
    }

    /// Rows the text takes as laid out, wrapped lines counted.
    pub(super) fn rows(&self, cx: &App) -> usize {
        each!(self, |state| state.read(cx).display_map().wrap_row_count()).max(1)
    }

    pub(super) fn set_language(&self, language: &str, cx: &mut App) {
        if let Self::Code(state) = self {
            let language = SharedString::from(language.to_owned());
            state.update(cx, |state, cx| state.set_highlighter(language, cx));
        }
    }

    /// Replaces `range` with `text`, keeping the chips outside it.
    pub(super) fn replace_range(
        &self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = SharedString::from(text.to_owned());
        each!(self, |state| state.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(text, window, cx);
        }))
    }

    /// Sets the text, the chips that fall in it at their places, and the caret. A code editor
    /// keeps the chips' text.
    pub(super) fn set_text(
        &self,
        text: &str,
        tokens: Vec<(Range<usize>, InlineToken)>,
        caret: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Text(state) => state.update(cx, |input, cx| {
                input.set_value(text.to_owned(), window, cx);
                for (range, token) in tokens {
                    input
                        .replace_range_with_token(range, token, window, cx)
                        .ok();
                }
                input.set_selected_range(caret..caret, cx);
            }),
            Self::Code(state) => state.update(cx, |input, cx| {
                input.set_value(text.to_owned(), window, cx);
                input.set_selected_range(caret..caret, cx);
            }),
        }
    }
}
