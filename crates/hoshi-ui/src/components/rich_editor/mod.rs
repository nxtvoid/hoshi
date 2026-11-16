//! A rich text editor made of blocks: paragraphs, headings, quotes, list items, fenced code
//! and rules, each one a text input of its own. It reads and writes markdown, turns markdown
//! typed at the start of a paragraph into its block, and moves the caret between blocks as one
//! document would.

mod block_input;
mod document;
mod languages;
mod marks;

use block_input::BlockInput;
pub use document::{
    BlockData, BlockKind, BlockSnapshot, ChipData, fence_language, from_markdown, shortcut,
    to_markdown,
};
pub use languages::{
    CODE_LANGUAGES, CodeLanguage, code_language_icon, code_language_id, code_language_label,
    search_code_languages,
};
pub use marks::{Mark, mark_shortcut};

use crate::components::{Align, Popover, PopoverChange, PopoverFocus, Side};
use crate::theme::ActiveTheme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    Editor, InlineToken, InlineTokenContext, InlineTokenSpan, Input, InputContent, InputEvent,
    InputState, TextDecoration, Textarea, TextareaState,
};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::ops::Range;
use std::rc::Rc;

type TokenRenderer = Rc<dyn Fn(&InlineTokenContext, &mut Window, &mut App) -> AnyElement>;
type TokenClick = Rc<dyn Fn(&InlineToken, &mut Window, &mut App)>;
type PasteHandler = Rc<dyn Fn(&ClipboardItem, &mut Window, &mut App) -> bool>;

/// What the editor tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichEditorEvent {
    /// The text, a block's kind or the caret changed.
    Change,
    /// Enter outside a code block, or Ctrl+Enter anywhere.
    Submit,
}

struct Block {
    id: u64,
    kind: BlockKind,
    input: BlockInput,
    _subscription: Subscription,
    /// Redraws the editor when the caret moves, which folds or unfolds the marks around it.
    _caret: Option<Subscription>,
}

pub struct RichEditor {
    blocks: Vec<Block>,
    active: usize,
    next_id: u64,
    placeholder: SharedString,
    /// The placeholder the first block shows now: `placeholder` while the document is empty.
    shown_placeholder: SharedString,
    /// The block showing it; a new first block starts with none.
    placeholder_owner: Option<EntityId>,
    token_renderer: Option<TokenRenderer>,
    paste_handler: Option<PasteHandler>,
    token_click: Option<TokenClick>,
    /// The code block whose language picker is open, by block id.
    picking_language: Option<u64>,
    /// Ctrl+A selected the whole document, past the block the caret is in.
    all_selected: bool,
    language_search: Entity<InputState>,
    labels: LanguageLabels,
    /// Room around the text: horizontal, vertical.
    inset: (Pixels, Pixels),
    _language_search: Subscription,
}

/// The language picker's text, from the app.
#[derive(Clone)]
pub struct LanguageLabels {
    pub plain: SharedString,
    pub none: SharedString,
}

impl Default for LanguageLabels {
    fn default() -> Self {
        Self {
            plain: "Plain text".into(),
            none: "No matching languages".into(),
        }
    }
}

impl EventEmitter<RichEditorEvent> for RichEditor {}

impl RichEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search languages"));
        let search_events = cx.subscribe_in(
            &language_search,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.pick_first_language(window, cx),
                _ => {}
            },
        );
        let mut editor = Self {
            blocks: Vec::new(),
            active: 0,
            next_id: 0,
            placeholder: SharedString::default(),
            shown_placeholder: SharedString::default(),
            placeholder_owner: None,
            token_renderer: None,
            paste_handler: None,
            token_click: None,
            picking_language: None,
            all_selected: false,
            language_search,
            labels: LanguageLabels::default(),
            inset: (px(10.), px(8.)),
            _language_search: search_events,
        };
        let first = editor.new_block(BlockKind::Paragraph, window, cx);
        editor.blocks.push(first);
        editor
    }

    /// Shown in the first block while the document is empty.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        self.sync_placeholder(window, cx);
    }

    /// The language picker's text: its labels and its search field's placeholder.
    pub fn set_language_labels(
        &mut self,
        labels: LanguageLabels,
        search: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.labels = labels;
        let search = search.into();
        self.language_search
            .update(cx, |state, cx| state.set_placeholder(search, window, cx));
    }

    /// Room around the text, horizontal and vertical.
    pub fn set_inset(&mut self, x: Pixels, y: Pixels) {
        self.inset = (x, y);
    }

    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    /// How inline tokens (chips) render, in place of the default.
    pub fn set_token_renderer(
        &mut self,
        render: impl Fn(&InlineTokenContext, &mut Window, &mut App) -> AnyElement + 'static,
    ) {
        self.token_renderer = Some(Rc::new(render));
    }

    /// Sees each paste first; `true` keeps it from the editor.
    pub fn set_paste_handler(
        &mut self,
        handler: impl Fn(&ClipboardItem, &mut Window, &mut App) -> bool + 'static,
    ) {
        self.paste_handler = Some(Rc::new(handler));
    }

    /// Runs when a chip is clicked.
    pub fn set_token_click_handler(
        &mut self,
        handler: impl Fn(&InlineToken, &mut Window, &mut App) + 'static,
    ) {
        self.token_click = Some(Rc::new(handler));
    }

    fn new_block(&mut self, kind: BlockKind, window: &mut Window, cx: &mut Context<Self>) -> Block {
        self.next_id += 1;
        let id = self.next_id;
        let input = match &kind {
            BlockKind::Code(language) => BlockInput::code(language, window, cx),
            _ => BlockInput::text(window, cx),
        };
        let subscription = match &input {
            BlockInput::Text(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    this.on_input(id, event, window, cx)
                })
            }
            BlockInput::Code(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    this.on_input(id, event, window, cx)
                })
            }
        };
        let caret = input
            .as_text()
            .map(|state| cx.observe(state, |_, _, cx| cx.notify()));
        Block {
            id,
            kind,
            input,
            _subscription: subscription,
            _caret: caret,
        }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.blocks.iter().position(|block| block.id == id)
    }

    fn on_input(
        &mut self,
        id: u64,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        match event {
            InputEvent::Focus => {
                if index != self.active {
                    self.all_selected = false;
                }
                self.active = index;
                cx.emit(RichEditorEvent::Change);
                cx.notify();
            }
            InputEvent::Change => {
                self.active = index;
                self.apply_shortcut(index, window, cx);
                self.sync_placeholder(window, cx);
                cx.emit(RichEditorEvent::Change);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {}
            InputEvent::Blur => {}
        }
    }

    fn active_block(&self) -> &BlockInput {
        &self.blocks[self.active.min(self.blocks.len() - 1)].input
    }

    /// The prose block the caret is in; `None` in a code block, which takes no chips.
    pub fn active_input(&self) -> Option<&Entity<TextareaState>> {
        self.active_block().as_text()
    }

    /// Types `text` at the caret of the block it is in, code blocks included.
    pub fn insert_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.active_block().clone();
        let selected = input.selected_range(cx);
        input.replace_range(selected, text, window, cx);
    }

    pub fn active_kind(&self) -> &BlockKind {
        &self.blocks[self.active.min(self.blocks.len() - 1)].kind
    }

    /// Whether the caret is in the document's first block.
    pub fn in_first_block(&self) -> bool {
        self.active == 0
    }

    pub fn in_code_block(&self) -> bool {
        matches!(self.active_kind(), BlockKind::Code(_))
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.blocks
            .iter()
            .any(|block| block.input.is_focused(window, cx))
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_block().focus(window, cx);
    }

    pub fn blocks(&self, cx: &App) -> Vec<BlockData> {
        self.blocks
            .iter()
            .map(|block| BlockData::new(block.kind.clone(), block.input.value(cx).to_string()))
            .collect()
    }

    /// The blocks with their chips, for [`RichEditor::restore`].
    pub fn snapshot(&self, cx: &App) -> Vec<BlockSnapshot> {
        self.blocks
            .iter()
            .map(|block| {
                let input = &block.input;
                BlockSnapshot {
                    kind: block.kind.clone(),
                    text: input.value(cx).to_string(),
                    chips: input
                        .tokens(cx)
                        .iter()
                        .map(|span| ChipData {
                            range: span.range(),
                            id: span.token().id().to_string(),
                            text: span.token().text().to_string(),
                            label: span.token().label().to_string(),
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Shows blocks taken with [`RichEditor::snapshot`], chips and all, the caret at the end.
    pub fn restore(
        &mut self,
        blocks: Vec<BlockSnapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if blocks.is_empty() {
            self.set_markdown("", window, cx);
            return;
        }
        let focused = self.is_focused(window, cx);
        self.all_selected = false;
        self.blocks.clear();
        for item in blocks {
            let block = self.new_block(item.kind, window, cx);
            let caret = item.text.len();
            let chips = item
                .chips
                .into_iter()
                .filter(|chip| {
                    chip.range.end <= item.text.len()
                        && item.text.get(chip.range.clone()) == Some(chip.text.as_str())
                })
                .map(|chip| {
                    (
                        chip.range,
                        InlineToken::new(chip.id, chip.text).with_label(chip.label),
                    )
                })
                .collect();
            block.input.set_text(&item.text, chips, caret, window, cx);
            self.blocks.push(block);
        }
        self.active = self.blocks.len() - 1;
        self.picking_language = None;
        if focused {
            self.focus_block(self.active, Caret::End, window, cx);
        }
        self.sync_placeholder(window, cx);
        cx.emit(RichEditorEvent::Change);
        cx.notify();
    }

    /// The document as markdown, chips written as their text.
    pub fn markdown(&self, cx: &App) -> String {
        to_markdown(&self.blocks(cx))
    }

    pub fn is_empty(&self, cx: &App) -> bool {
        self.blocks.len() == 1
            && self.blocks[0].kind == BlockKind::Paragraph
            && self.blocks[0].input.value(cx).is_empty()
    }

    /// Shows `markdown` as blocks, the caret at its end. Chips come back as plain text.
    pub fn set_markdown(&mut self, markdown: &str, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.is_focused(window, cx);
        self.all_selected = false;
        let data = from_markdown(markdown);
        self.blocks.clear();
        for item in data {
            let block = self.new_block(item.kind, window, cx);
            let caret = item.text.len();
            block
                .input
                .set_text(&item.text, Vec::new(), caret, window, cx);
            self.blocks.push(block);
        }
        self.active = self.blocks.len() - 1;
        self.picking_language = None;
        if focused {
            self.focus_block(self.active, Caret::End, window, cx);
        }
        self.sync_placeholder(window, cx);
        cx.emit(RichEditorEvent::Change);
        cx.notify();
    }

    /// Every chip of the document, block by block.
    pub fn tokens(&self, cx: &App) -> Vec<InlineTokenSpan> {
        self.blocks
            .iter()
            .flat_map(|block| block.input.tokens(cx))
            .collect()
    }

    /// Takes the chips with this id out of the text.
    pub fn remove_tokens(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        for block in &self.blocks {
            let Some(input) = block.input.as_text() else {
                continue;
            };
            input.update(cx, |input, cx| {
                let ranges: Vec<_> = input
                    .tokens()
                    .iter()
                    .filter(|span| span.token().id().as_ref() == id)
                    .map(|span| span.range())
                    .collect();
                for range in ranges.into_iter().rev() {
                    input.set_selected_range(range, cx);
                    input.replace("", window, cx);
                }
            });
        }
    }

    fn sync_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = if self.is_empty(cx) {
            self.placeholder.clone()
        } else {
            SharedString::default()
        };
        let first = self.blocks[0].input.clone();
        if self.shown_placeholder != shown || Some(first.entity_id()) != self.placeholder_owner {
            self.shown_placeholder = shown.clone();
            self.placeholder_owner = Some(first.entity_id());
            first.set_placeholder(shown, window, cx);
        }
    }

    /// The chips of `input` inside `range`, shifted by `shift` bytes.
    fn tokens_in(
        input: &BlockInput,
        range: Range<usize>,
        shift: isize,
        cx: &App,
    ) -> Vec<(Range<usize>, InlineToken)> {
        input
            .tokens(cx)
            .iter()
            .filter(|span| span.range().start >= range.start && span.range().end <= range.end)
            .map(|span| {
                let moved = span.range();
                let start = (moved.start as isize + shift) as usize;
                let end = (moved.end as isize + shift) as usize;
                (start..end, span.token().clone())
            })
            .collect()
    }

    fn apply_shortcut(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocks[index].kind != BlockKind::Paragraph {
            return;
        }
        let input = self.blocks[index].input.clone();
        let (text, cursor) = (input.value(cx).to_string(), input.cursor(cx));
        let Some((kind, rest)) = shortcut(&text) else {
            return;
        };
        let marker = text.len() - rest.len();
        if cursor < marker {
            return;
        }
        let tokens = Self::tokens_in(&input, marker..text.len(), -(marker as isize), cx);
        let rest = rest.to_owned();
        if kind == BlockKind::Rule {
            self.blocks[index].kind = BlockKind::Rule;
            input.set_text("", Vec::new(), 0, window, cx);
            if index + 1 >= self.blocks.len() || !self.blocks[index + 1].kind.has_text() {
                let next = self.new_block(BlockKind::Paragraph, window, cx);
                self.blocks.insert(index + 1, next);
            }
            self.focus_block(index + 1, Caret::Start, window, cx);
            return;
        }
        self.blocks[index].kind = kind;
        input.set_text(&rest, tokens, cursor - marker, window, cx);
    }

    /// Gives the block at `index` another kind, moving its text to the input that kind needs.
    fn set_kind(
        &mut self,
        index: usize,
        kind: BlockKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let code = matches!(kind, BlockKind::Code(_));
        let input = self.blocks[index].input.clone();
        if code == input.is_code() {
            if let BlockKind::Code(language) = &kind {
                input.set_language(language, cx);
            }
            self.blocks[index].kind = kind;
            return;
        }
        let text = input.value(cx).to_string();
        let caret = input.cursor(cx);
        let tokens = Self::tokens_in(&input, 0..text.len(), 0, cx);
        let focused = input.is_focused(window, cx);
        let block = self.new_block(kind, window, cx);
        block.input.set_text(&text, tokens, caret, window, cx);
        self.blocks[index] = block;
        if focused {
            self.focus_block(index, Caret::At(caret), window, cx);
        }
    }

    /// Turns the paragraph at `index` into a code block when it holds only an opening fence.
    fn open_fence(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.blocks[index].kind != BlockKind::Paragraph {
            return false;
        }
        let text = self.blocks[index].input.value(cx).to_string();
        let Some(language) = fence_language(&text) else {
            return false;
        };
        let block = self.new_block(BlockKind::Code(language), window, cx);
        self.blocks[index] = block;
        if index + 1 == self.blocks.len() {
            let after = self.new_block(BlockKind::Paragraph, window, cx);
            self.blocks.push(after);
        }
        self.focus_block(index, Caret::Start, window, cx);
        self.changed(window, cx);
        true
    }

    fn focus_block(
        &mut self,
        index: usize,
        caret: Caret,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = index.min(self.blocks.len() - 1);
        if !self.blocks[index].kind.has_text() {
            return;
        }
        self.active = index;
        let input = self.blocks[index].input.clone();
        let length = input.value(cx).len();
        let at = match caret {
            Caret::Start => 0,
            Caret::End => length,
            Caret::At(at) => at.min(length),
        };
        input.set_selected_range(at..at, cx);
        input.focus(window, cx);
    }

    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_placeholder(window, cx);
        cx.emit(RichEditorEvent::Change);
        cx.notify();
    }

    /// Splits the block at the caret, the part after it going to a new block below.
    fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.active;
        let kind = self.blocks[index].kind.clone();
        let input = self.blocks[index].input.clone();
        let (text, cursor) = (input.value(cx).to_string(), input.cursor(cx));
        let list = matches!(
            kind,
            BlockKind::Bullet | BlockKind::Ordered | BlockKind::Quote
        );
        if list && text.is_empty() {
            self.set_kind(index, BlockKind::Paragraph, window, cx);
            self.changed(window, cx);
            return;
        }
        let next_kind = if list { kind } else { BlockKind::Paragraph };
        let kept = Self::tokens_in(&input, 0..cursor, 0, cx);
        let moved = Self::tokens_in(&input, cursor..text.len(), -(cursor as isize), cx);
        input.set_text(&text[..cursor], kept, cursor, window, cx);
        let next = self.new_block(next_kind, window, cx);
        next.input.set_text(&text[cursor..], moved, 0, window, cx);
        self.blocks.insert(index + 1, next);
        self.focus_block(index + 1, Caret::Start, window, cx);
        self.changed(window, cx);
    }

    /// Joins the block at `index` onto the one before it.
    fn merge_into_previous(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == 0 {
            return;
        }
        if !self.blocks[index - 1].kind.has_text() {
            self.blocks.remove(index - 1);
            self.focus_block(index - 1, Caret::Start, window, cx);
            self.changed(window, cx);
            return;
        }
        let previous = self.blocks[index - 1].input.clone();
        let current = self.blocks[index].input.clone();
        let head = previous.value(cx).to_string();
        let tail = current.value(cx).to_string();
        let mut tokens = Self::tokens_in(&previous, 0..head.len(), 0, cx);
        tokens.extend(Self::tokens_in(
            &current,
            0..tail.len(),
            head.len() as isize,
            cx,
        ));
        let joined = format!("{head}{tail}");
        previous.set_text(&joined, tokens, head.len(), window, cx);
        self.blocks.remove(index);
        self.focus_block(index - 1, Caret::At(head.len()), window, cx);
        self.changed(window, cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.all_selected = true;
        for block in &self.blocks {
            let length = block.input.value(cx).len();
            block.input.set_selected_range(0..length, cx);
        }
        cx.notify();
    }

    /// Empties the document into the block the caret is in, which keeps the focus.
    fn clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.all_selected = false;
        let index = self.active.min(self.blocks.len() - 1);
        let kept = self.blocks.remove(index);
        self.blocks.clear();
        self.blocks.push(kept);
        self.active = 0;
        let input = self.blocks[0].input.clone();
        input.set_text("", Vec::new(), 0, window, cx);
        self.set_kind(0, BlockKind::Paragraph, window, cx);
        self.changed(window, cx);
    }

    /// Keys that act on the whole document while Ctrl+A has it selected; `None` lets the key
    /// go on, the selection dropped.
    fn whole_document_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let modifiers = &keystroke.modifiers;
        let control = modifiers.control || modifiers.platform;
        match keystroke.key.as_str() {
            "backspace" | "delete" if !control && !modifiers.alt => {
                self.clear_all(window, cx);
                Some(true)
            }
            "a" if control => Some(true),
            "c" | "x" if control && !modifiers.shift => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.markdown(cx)));
                if keystroke.key == "x" {
                    self.clear_all(window, cx);
                }
                Some(true)
            }
            "shift" | "control" | "alt" | "platform" | "function" => Some(false),
            _ => {
                self.all_selected = false;
                let typed = keystroke.key_char.is_some() && !control && !modifiers.alt;
                if typed {
                    self.clear_all(window, cx);
                }
                cx.notify();
                None
            }
        }
    }

    /// Whether the caret is on the document's first line as shown.
    pub fn on_first_line(&self, cx: &App) -> bool {
        self.active == 0 && self.caret_line(cx).0
    }

    /// Whether the caret is on the document's last line as shown.
    pub fn on_last_line(&self, cx: &App) -> bool {
        self.active + 1 >= self.blocks.len() && self.caret_line(cx).1
    }

    /// Where the caret sits in its block: on the first or the last line as shown.
    fn caret_line(&self, cx: &App) -> (bool, bool) {
        let input = self.active_block();
        let text = input.value(cx);
        let cursor = input.cursor(cx);
        let Some((caret, line_height, bounds)) = input.caret_geometry(cx) else {
            return (
                !text[..cursor].contains('\n'),
                !text[cursor..].contains('\n'),
            );
        };
        let first = caret.top() < bounds.top() + line_height * 0.5;
        let last = caret.bottom() > bounds.bottom() - line_height * 0.5;
        (first, last)
    }

    /// Keys that move across blocks or shape them. The owner calls this from its keystroke
    /// interceptor, after its own menus had their turn; `true` when the key was used.
    pub fn handle_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_focused(window, cx) {
            return false;
        }
        if self.all_selected
            && let Some(handled) = self.whole_document_key(keystroke, window, cx)
        {
            return handled;
        }
        let modifiers = &keystroke.modifiers;
        let index = self.active.min(self.blocks.len() - 1);
        let code = matches!(self.blocks[index].kind, BlockKind::Code(_));
        let (cursor, length, selected) = {
            let input = &self.blocks[index].input;
            (
                input.cursor(cx),
                input.value(cx).len(),
                !input.selected_range(cx).is_empty(),
            )
        };
        if !code
            && let Some(mark) = mark_shortcut(
                &keystroke.key,
                modifiers.control,
                modifiers.platform,
                modifiers.alt,
                modifiers.shift,
            )
        {
            self.toggle_mark(index, mark, window, cx);
            return true;
        }
        let control = modifiers.control || modifiers.platform;
        match keystroke.key.as_str() {
            "enter" if control && !modifiers.alt => {
                cx.emit(RichEditorEvent::Submit);
                true
            }
            "enter" if modifiers.alt => false,
            "enter" if modifiers.shift => {
                if code {
                    return false;
                }
                if !self.open_fence(index, window, cx) {
                    self.split(window, cx);
                }
                true
            }
            "enter" => {
                if code {
                    return false;
                }
                if !self.open_fence(index, window, cx) {
                    cx.emit(RichEditorEvent::Submit);
                }
                true
            }
            "a" if control && !modifiers.shift && !modifiers.alt && self.blocks.len() > 1 => {
                self.select_all(cx);
                true
            }
            _ if control || modifiers.alt => false,
            "backspace" if cursor == 0 && !selected => {
                if self.blocks[index].kind != BlockKind::Paragraph {
                    let code_with_text = code && length > 0;
                    if code_with_text {
                        return false;
                    }
                    self.set_kind(index, BlockKind::Paragraph, window, cx);
                    self.changed(window, cx);
                    return true;
                }
                if index == 0 {
                    return false;
                }
                self.merge_into_previous(index, window, cx);
                true
            }
            "delete" if cursor == length && !selected && index + 1 < self.blocks.len() => {
                self.merge_into_previous(index + 1, window, cx);
                true
            }
            "up" if !modifiers.shift && index > 0 && self.caret_line(cx).0 => {
                let target = (0..index).rev().find(|at| self.blocks[*at].kind.has_text());
                match target {
                    Some(target) => {
                        self.focus_block(target, Caret::End, window, cx);
                        true
                    }
                    None => false,
                }
            }
            "down"
                if !modifiers.shift && index + 1 < self.blocks.len() && self.caret_line(cx).1 =>
            {
                let target =
                    (index + 1..self.blocks.len()).find(|at| self.blocks[*at].kind.has_text());
                match target {
                    Some(target) => {
                        self.focus_block(target, Caret::Start, window, cx);
                        true
                    }
                    None => false,
                }
            }
            "left" if !modifiers.shift && cursor == 0 && !selected && index > 0 => {
                let target = (0..index).rev().find(|at| self.blocks[*at].kind.has_text());
                target.is_some_and(|target| {
                    self.focus_block(target, Caret::End, window, cx);
                    true
                })
            }
            "right" if !modifiers.shift && cursor == length && !selected => {
                let target =
                    (index + 1..self.blocks.len()).find(|at| self.blocks[*at].kind.has_text());
                target.is_some_and(|target| {
                    self.focus_block(target, Caret::Start, window, cx);
                    true
                })
            }
            _ => false,
        }
    }

    /// Wraps the selection of the block at `index` in `mark`'s markers, or unwraps it.
    fn toggle_mark(
        &mut self,
        index: usize,
        mark: Mark,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.blocks[index].input.clone();
        let text = input.value(cx).to_string();
        let (edits, selection) = marks::toggle(&text, input.selected_range(cx), mark);
        for (range, replacement) in edits {
            input.replace_range(range, replacement, window, cx);
        }
        input.set_selected_range(selection, cx);
        self.changed(window, cx);
    }

    fn set_language(
        &mut self,
        id: u64,
        language: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picking_language = None;
        if let Some(index) = self.index_of(id) {
            self.set_kind(index, BlockKind::Code(language), window, cx);
            self.focus_block(index, Caret::End, window, cx);
        }
        self.changed(window, cx);
    }

    fn language_options(&self, cx: &App) -> Vec<(&'static str, SharedString)> {
        let query = self.language_search.read(cx).value().to_string();
        let plain = query
            .trim()
            .is_empty()
            .then(|| ("", self.labels.plain.clone()));
        plain
            .into_iter()
            .chain(
                search_code_languages(&query)
                    .into_iter()
                    .map(|language| (language.id, SharedString::from(language.label))),
            )
            .collect()
    }

    fn pick_first_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.language_options(cx).first().map(|(id, _)| *id);
        if let (Some(id), Some(language)) = (self.picking_language, first) {
            self.set_language(id, language.to_owned(), window, cx);
        }
    }

    fn open_language_picker(
        &mut self,
        id: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picking_language = id;
        self.language_search
            .update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
    }

    fn render_language_picker(
        &self,
        id: u64,
        language: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let label: SharedString = if language.is_empty() {
            self.labels.plain.clone()
        } else {
            code_language_label(language).into()
        };
        let current = code_language_id(language);
        let open = self.picking_language == Some(id);
        let options = if open {
            self.language_options(cx)
        } else {
            Vec::new()
        };
        let none = self.labels.none.clone();
        let search = self.language_search.clone();
        let search_focus = search.read(cx).focus_handle(cx);
        let this = cx.weak_entity();
        let muted = colors.muted_foreground;
        let foreground = colors.foreground;
        let (icon, tint) = code_language_icon(language);
        let icon_color = tint(&colors);
        Popover::new(("code-language", id))
            .side(Side::Bottom)
            .align(Align::Start)
            .modal(false)
            .w_56()
            .p_1()
            .gap_1()
            .initial_focus(PopoverFocus::Handle(search_focus))
            .open(open)
            .on_open_change(
                cx.listener(move |this, change: &PopoverChange, window, cx| {
                    this.open_language_picker(change.open.then_some(id), window, cx)
                }),
            )
            .trigger(move |open| {
                div()
                    .id(("code-language-trigger", id))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h_6()
                    .px_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if open { foreground } else { muted })
                    .hover(move |style| style.text_color(foreground))
                    .child(
                        svg()
                            .path(crate::assets::file_icon(icon))
                            .size_3p5()
                            .flex_none()
                            .text_color(icon_color),
                    )
                    .child(div().max_w_40().truncate().child(label))
                    .child(Icon::new(crate::assets::IconName::ChevronDown).size_3())
            })
            .content(move |_, _, cx| {
                let colors = cx.hoshi().clone();
                let rows: Vec<AnyElement> = options
                    .into_iter()
                    .map(|(language, label)| {
                        let this = this.clone();
                        let selected = language == current;
                        let (icon, tint) = code_language_icon(language);
                        div()
                            .id(SharedString::from(format!("code-language-{language}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(colors.popover_foreground)
                            .when(selected, |row| row.font_weight(FontWeight::MEDIUM))
                            .hover(|style| {
                                style.bg(colors.accent).text_color(colors.accent_foreground)
                            })
                            .child(
                                svg()
                                    .path(crate::assets::file_icon(icon))
                                    .size_3p5()
                                    .flex_none()
                                    .text_color(tint(&colors)),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            .when(selected, |row| {
                                row.child(
                                    Icon::new(crate::assets::IconName::Check)
                                        .size_3p5()
                                        .flex_none(),
                                )
                            })
                            .on_click(move |_, window, cx| {
                                this.update(cx, |this, cx| {
                                    this.set_language(id, language.to_owned(), window, cx)
                                })
                                .ok();
                            })
                            .into_any_element()
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .pb_1()
                            .border_b_1()
                            .border_color(colors.border)
                            .child(
                                Icon::new(crate::assets::IconName::Search)
                                    .size_3p5()
                                    .flex_none()
                                    .text_color(colors.muted_foreground),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .child(Input::new(&search).appearance(false)),
                            ),
                    )
                    .child(if rows.is_empty() {
                        div()
                            .px_2()
                            .py_1p5()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(none)
                            .into_any_element()
                    } else {
                        div()
                            .id(("code-language-rows", id))
                            .max_h_64()
                            .overflow_y_scroll()
                            .children(rows)
                            .into_any_element()
                    })
            })
            .into_any_element()
    }
}

impl RichEditor {
    fn render_text(
        &self,
        state: &Entity<TextareaState>,
        kind: &BlockKind,
        own_font: Option<&(SharedString, Pixels)>,
    ) -> Textarea {
        let mut textarea = Textarea::new(state)
            .appearance(false)
            .xsmall()
            .ml(px(-4.))
            .mr(px(-4.))
            .map(|this| match (own_font, kind) {
                (Some((_, size)), kind) => this.text_size(*size * prompt_scale(kind)),
                (None, BlockKind::Heading(1)) => this.text_xl(),
                (None, BlockKind::Heading(2)) => this.text_lg(),
                (None, BlockKind::Heading(_)) => this.text_base(),
                (None, BlockKind::Code(_)) => this.text_xs(),
                (None, _) => this.text_sm(),
            });
        let render = self.token_renderer.clone();
        textarea = textarea.token(move |context, window, cx| {
            if let Some(mark) = mark_of(context.token()) {
                return render_mark(mark, context.token().label().clone(), cx);
            }
            match &render {
                Some(render) => render(context, window, cx),
                None => div()
                    .child(context.token().label().clone())
                    .into_any_element(),
            }
        });
        if let Some(paste) = self.paste_handler.clone() {
            textarea = textarea.on_paste(move |item, window, cx| paste(item, window, cx));
        }
        if let Some(click) = self.token_click.clone() {
            textarea = textarea.on_token_click(move |event, window, cx| {
                if !is_mark_token(event.token()) {
                    click(event.token(), window, cx)
                }
            });
        }
        textarea
    }
}

#[derive(Clone, Copy)]
enum Caret {
    Start,
    End,
    At(usize),
}

/// A block's text size against the prompt's own size, as the interface's sizes stand against
/// `text-sm`.
fn prompt_scale(kind: &BlockKind) -> f32 {
    match kind {
        BlockKind::Heading(1) => 1.25 / 0.875,
        BlockKind::Heading(2) => 1.125 / 0.875,
        BlockKind::Heading(_) => 1. / 0.875,
        BlockKind::Code(_) => 0.75 / 0.875,
        _ => 1.,
    }
}

const MARK_TOKEN: &str = "mark:";

pub(super) fn is_mark_token(token: &InlineToken) -> bool {
    token.id().starts_with(MARK_TOKEN)
}

fn mark_of(token: &InlineToken) -> Option<Mark> {
    match token.id().strip_prefix(MARK_TOKEN)?.split(':').next()? {
        "bold" => Some(Mark::Bold),
        "italic" => Some(Mark::Italic),
        "code" => Some(Mark::Code),
        "strike" => Some(Mark::Strike),
        _ => None,
    }
}

fn mark_name(mark: Mark) -> &'static str {
    match mark {
        Mark::Bold => "bold",
        Mark::Italic => "italic",
        Mark::Code => "code",
        Mark::Strike => "strike",
    }
}

/// A mark folded away from the caret: its text styled, without its markers.
fn render_mark(mark: Mark, text: SharedString, cx: &App) -> AnyElement {
    let colors = cx.hoshi();
    div()
        .flex_none()
        .whitespace_nowrap()
        .map(|this| match mark {
            Mark::Bold => this.font_weight(FontWeight::BOLD),
            Mark::Italic => this.italic(),
            Mark::Code => this.px_0p5().rounded_sm().bg(colors.muted),
            Mark::Strike => this.line_through(),
        })
        .child(text)
        .into_any_element()
}

/// Splits a block's marks: those the caret is away from fold into tokens that show their text
/// styled, those it touches stay text with their markers faint, as the web composer does. Chips
/// keep their places; a mark that would cut through one stays text.
fn fold_marks(
    state: &TextareaState,
    focused: bool,
    colors: &crate::theme::Colors,
) -> (Vec<TextDecoration>, InputContent) {
    let text = state.value();
    let selection = state.selected_range();
    let chips: Vec<InlineTokenSpan> = state
        .tokens()
        .iter()
        .filter(|span| !is_mark_token(span.token()))
        .cloned()
        .collect();
    let mut decorations = Vec::new();
    let mut folded: Vec<(Range<usize>, InlineToken)> = Vec::new();
    for span in marks::spans(&text) {
        let whole = span.open.start..span.close.end;
        let near = focused && selection.start <= whole.end && whole.start <= selection.end;
        let crosses_chip = chips
            .iter()
            .any(|chip| chip.range().start < whole.end && whole.start < chip.range().end);
        if !near && !crosses_chip {
            let (words, gaps) = marks::fold_pieces(&text, &span);
            for word in words {
                let label = word.start.max(span.content.start)..word.end.min(span.content.end);
                let token = InlineToken::new(
                    format!("{MARK_TOKEN}{}:{}", mark_name(span.mark), word.start),
                    text[word.clone()].to_owned(),
                )
                .with_label(text[label].to_owned());
                folded.push((word, token));
            }
            let style = mark_style(span.mark, colors);
            decorations.extend(gaps.into_iter().map(|gap| TextDecoration::new(gap, style)));
            continue;
        }
        decorations.extend(mark_decorations(&span, colors));
    }
    let mut tokens: Vec<(Range<usize>, InlineToken)> = chips
        .into_iter()
        .map(|span| (span.range(), span.token().clone()))
        .chain(folded)
        .collect();
    tokens.sort_by_key(|(range, _)| range.start);
    let mut content = InputContent::new(text.clone());
    for (range, token) in tokens {
        if let Ok(next) = content.clone().with_token(range, token) {
            content = next;
        }
    }
    (decorations, content)
}

/// How a mark the caret touches shows: its text styled, its markers faint.
fn mark_decorations(span: &marks::MarkSpan, colors: &crate::theme::Colors) -> Vec<TextDecoration> {
    let faint = HighlightStyle {
        color: Some(colors.muted_foreground.opacity(0.6)),
        ..HighlightStyle::default()
    };
    let style = mark_style(span.mark, colors);
    vec![
        TextDecoration::new(span.open.clone(), faint),
        TextDecoration::new(span.content.clone(), style),
        TextDecoration::new(span.close.clone(), faint),
    ]
}

fn mark_style(mark: Mark, colors: &crate::theme::Colors) -> HighlightStyle {
    match mark {
        Mark::Bold => HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..HighlightStyle::default()
        },
        Mark::Italic => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..HighlightStyle::default()
        },
        Mark::Code => HighlightStyle {
            background_color: Some(colors.muted),
            ..HighlightStyle::default()
        },
        Mark::Strike => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            ..HighlightStyle::default()
        },
    }
}

impl Render for RichEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let mono = cx.theme().mono_font_family.clone();
        let own_font = crate::theme::fonts(cx).composer;
        let mut number = 0;
        let mut children: Vec<AnyElement> = Vec::with_capacity(self.blocks.len());
        for index in 0..self.blocks.len() {
            let block = &self.blocks[index];
            let previous = index.checked_sub(1).map(|at| &self.blocks[at].kind);
            if previous != Some(&BlockKind::Ordered) {
                number = 0;
            }
            let textarea = match &block.input {
                BlockInput::Text(state) => {
                    let focused = state.read(cx).focus_handle(cx).is_focused(window);
                    let (decorations, content) = fold_marks(state.read(cx), focused, &colors);
                    state.update(cx, |state, cx| {
                        state.set_decorations(decorations, cx);
                        state.set_tokens(content, cx).ok();
                    });
                    self.render_text(state, &block.kind, own_font.as_ref())
                        .into_any_element()
                }
                BlockInput::Code(state) => {
                    let size = match &own_font {
                        Some((_, size)) => *size * prompt_scale(&block.kind),
                        None => window.rem_size() * 0.75,
                    };
                    let line_height = window.rem_size() * 1.25;
                    let height = line_height * block.input.rows(cx) as f32;
                    let mut editor = Editor::new(state)
                        .appearance(false)
                        .xsmall()
                        .ml(px(-4.))
                        .mr(px(-4.))
                        .text_size(size)
                        .line_height(line_height)
                        .h(height);
                    if let Some(paste) = self.paste_handler.clone() {
                        editor = editor.on_paste(move |item, window, cx| paste(item, window, cx));
                    }
                    div()
                        .h(height)
                        .overflow_hidden()
                        .child(editor)
                        .into_any_element()
                }
            };
            let first_of_run = previous != Some(&block.kind);
            let last_of_run =
                self.blocks.get(index + 1).map(|next| &next.kind) != Some(&block.kind);
            let element = match &block.kind {
                BlockKind::Paragraph => div().child(textarea).into_any_element(),
                BlockKind::Heading(_) => div()
                    .my_0p5()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(textarea)
                    .into_any_element(),
                BlockKind::Quote => div()
                    .when(first_of_run, |this| this.mt_1())
                    .when(last_of_run, |this| this.mb_1())
                    .border_l_2()
                    .border_color(colors.foreground.opacity(0.25))
                    .pl_3()
                    .text_color(colors.muted_foreground)
                    .child(textarea)
                    .into_any_element(),
                BlockKind::Bullet | BlockKind::Ordered => {
                    let marker = if block.kind == BlockKind::Bullet {
                        "•".to_owned()
                    } else {
                        number += 1;
                        format!("{number}.")
                    };
                    div()
                        .when(first_of_run, |this| this.mt_1())
                        .when(last_of_run, |this| this.mb_1())
                        .flex()
                        .items_start()
                        .child(
                            div()
                                .flex_none()
                                .w_6()
                                .pl_1()
                                .text_color(colors.muted_foreground)
                                .child(marker),
                        )
                        .child(div().flex_1().min_w_0().child(textarea))
                        .into_any_element()
                }
                BlockKind::Code(language) => div()
                    .my_1()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border.opacity(0.7))
                    .bg(colors.secondary.opacity(0.6))
                    .child(
                        div()
                            .h_7()
                            .flex()
                            .items_center()
                            .px_1()
                            .child(self.render_language_picker(block.id, language, cx)),
                    )
                    .child(
                        div()
                            .px_3()
                            .pb_2()
                            .font_family(mono.clone())
                            .text_xs()
                            .child(textarea),
                    )
                    .into_any_element(),
                BlockKind::Rule => div().my_2().h_px().bg(colors.border).into_any_element(),
            };
            let element = if self.all_selected && index != self.active {
                div()
                    .rounded_sm()
                    .bg(cx.theme().selection)
                    .child(element)
                    .into_any_element()
            } else {
                element
            };
            children.push(element);
        }
        div()
            .flex()
            .flex_col()
            .px(self.inset.0)
            .py(self.inset.1)
            .when_some(own_font, |this, (family, _)| this.font_family(family))
            .children(children)
    }
}

#[cfg(test)]
mod tests;
