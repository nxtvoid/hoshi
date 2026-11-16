mod actions;
mod header;
mod pickers;
mod summary;
mod timeline;

use super::changes::{ChangedFiles, Files, Source, totals};
use super::panel_message;
use crate::m;
use crate::state::Workspace;
pub use actions::PullRequestForge;
use actions::forge_error_message;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::*;
use hoshi_core::{
    ChecksState, PullRequest, PullRequestAction, PullRequestCheck, PullRequestDetail,
    PullRequestState, ReviewState, ReviewThread,
};
use hoshi_ui::prelude::*;
use pickers::{LabelPicker, ReviewerPicker};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Summary,
    Timeline,
    Code,
}

const TABS: [Tab; 3] = [Tab::Summary, Tab::Timeline, Tab::Code];

impl Tab {
    fn label(self) -> String {
        match self {
            Self::Summary => m::pr_tab_summary(),
            Self::Timeline => m::pr_tab_timeline(),
            Self::Code => m::pr_tab_code(),
        }
    }
}

/// A text box being written in the surface.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Editing {
    Title,
    Body,
    /// A reply to the review thread of that id.
    Reply(String),
    Comment,
}

impl Editing {
    fn key(&self) -> String {
        match self {
            Self::Title => "title".into(),
            Self::Body => "body".into(),
            Self::Reply(id) => format!("reply-{id}"),
            Self::Comment => "comment".into(),
        }
    }
}

/// A multi-line box whose buttons follow what it holds.
struct TextBox {
    state: Entity<TextareaState>,
    _changes: Subscription,
}

/// What changes when the forge's pull request changes, so the detail is read again.
pub fn version(pull_request: &PullRequest) -> String {
    format!(
        "{}|{}|{:?}|{:?}|{}",
        pull_request.number,
        pull_request.updated_at,
        pull_request.state,
        pull_request.checks,
        pull_request.unresolved_comments
    )
}

pub fn state_look(state: PullRequestState, colors: &Colors) -> (IconName, Hsla, String) {
    match state {
        PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open, m::pr_state_open()),
        PullRequestState::Draft => (
            IconName::GitPullRequestDraft,
            colors.muted_foreground,
            m::pr_state_draft(),
        ),
        PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged, m::pr_state_merged()),
        PullRequestState::Closed => (
            IconName::GitPullRequestClosed,
            colors.status_error,
            m::pr_state_closed(),
        ),
    }
}

fn check_look(state: ChecksState, colors: &Colors) -> (IconName, Hsla) {
    match state {
        ChecksState::Passing => (IconName::CircleCheck, colors.pr_open),
        ChecksState::Failing => (IconName::CircleX, colors.status_error),
        ChecksState::Pending => (IconName::Clock, colors.status_waiting),
    }
}

fn review_look(state: ReviewState, colors: &Colors) -> (IconName, Hsla, String) {
    match state {
        ReviewState::Requested => (
            IconName::Clock,
            colors.muted_foreground,
            m::review_requested(),
        ),
        ReviewState::Approved => (IconName::CircleCheck, colors.pr_open, m::review_approved()),
        ReviewState::ChangesRequested => (
            IconName::CircleX,
            colors.warning,
            m::review_changes_requested(),
        ),
        ReviewState::Commented => (
            IconName::MessageSquare,
            colors.muted_foreground,
            m::review_commented(),
        ),
        ReviewState::Dismissed => (
            IconName::MessageSquare,
            colors.muted_foreground,
            m::review_dismissed(),
        ),
    }
}

/// The overall state of the checks and how it reads: the failing ones first, then the
/// running ones.
pub fn checks_summary(checks: &[PullRequestCheck]) -> Option<(ChecksState, String)> {
    if checks.is_empty() {
        return None;
    }
    let count = |state| checks.iter().filter(|check| check.state == state).count();
    let (failing, pending) = (count(ChecksState::Failing), count(ChecksState::Pending));
    Some(if failing > 0 {
        (ChecksState::Failing, m::checks_failing_count(failing))
    } else if pending > 0 {
        (ChecksState::Pending, m::checks_pending_count(pending))
    } else {
        (ChecksState::Passing, m::checks_all_passed())
    })
}

/// The unresolved review threads as plain text for the agent: where each one is, then who
/// said what.
pub fn format_review_threads(threads: &[&ReviewThread]) -> String {
    threads
        .iter()
        .filter(|thread| !thread.resolved && !thread.comments.is_empty())
        .map(|thread| {
            let place = match thread.line {
                Some(line) => format!("{}:{line}", thread.path),
                None => thread.path.clone(),
            };
            let mut lines = vec![format!("- {place}")];
            lines.extend(thread.comments.iter().map(|comment| {
                let body = comment.body.trim().replace('\n', "\n    ");
                format!("  - {}: {body}", comment.author.as_deref().unwrap_or("?"))
            }));
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A label's color as the forge stores it: hex without `#`.
fn label_color(hex: &str) -> Option<Hsla> {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    (hex.trim_start_matches('#').len() == 6).then(|| rgb(value).into())
}

/// How long the checkout command shows as copied.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// A thread's pull request, laid out like the forge's own page, with its actions.
pub struct PullRequestSurface {
    workspace: Entity<Workspace>,
    thread_id: String,
    tab: Tab,
    version: Option<String>,
    code: Option<Entity<ChangedFiles>>,
    sending: bool,
    forge: Arc<dyn PullRequestForge>,
    /// A change is on its way to the forge.
    pending: bool,
    /// Why the last change failed, until dismissed or the next one.
    error: Option<String>,
    title: Option<(Entity<gpui_kit::component::input::InputState>, Subscription)>,
    body: Option<TextBox>,
    replies: HashMap<String, TextBox>,
    comment: Option<TextBox>,
    reviewers: ReviewerPicker,
    labels: LabelPicker,
    copied: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl PullRequestSurface {
    pub fn new(
        workspace: Entity<Workspace>,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&workspace, |this, _, cx| this.refresh(cx));
        let this = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = &event.keystroke;
            let modifiers = keystroke.modifiers;
            let submit = keystroke.key == "enter"
                && !modifiers.shift
                && !modifiers.alt
                && if cfg!(target_os = "macos") {
                    modifiers.platform
                } else {
                    modifiers.control
                };
            let escape = keystroke.key == "escape" && !modifiers.modified();
            if !submit && !escape {
                return;
            }
            let handled = this
                .update(cx, |this, cx| {
                    let Some(editing) = this.focused_editor(window, cx) else {
                        return false;
                    };
                    match (submit, &editing) {
                        (true, Editing::Title) => false,
                        (true, _) => {
                            this.save(editing, window, cx);
                            true
                        }
                        (false, Editing::Comment) => false,
                        (false, _) => {
                            this.cancel(&editing, cx);
                            true
                        }
                    }
                })
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let reviewers = ReviewerPicker::new(window, cx);
        let labels = LabelPicker::new(window, cx);
        let forge: Arc<dyn PullRequestForge> = workspace.read(cx).engine.clone();
        let mut this = Self {
            workspace,
            thread_id,
            tab: Tab::Summary,
            version: None,
            code: None,
            sending: false,
            forge,
            pending: false,
            error: None,
            title: None,
            body: None,
            replies: HashMap::new(),
            comment: None,
            reviewers,
            labels,
            copied: None,
            _subscriptions: vec![observe, keys],
        };
        this.refresh(cx);
        this
    }

    #[cfg(test)]
    pub fn tab(&self) -> Tab {
        self.tab
    }

    #[cfg(test)]
    pub fn set_forge(&mut self, forge: Arc<dyn PullRequestForge>) {
        self.forge = forge;
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    #[cfg(test)]
    pub fn is_editing(&self, editing: &Editing) -> bool {
        match editing {
            Editing::Title => self.title.is_some(),
            Editing::Body => self.body.is_some(),
            Editing::Reply(id) => self.replies.contains_key(id),
            Editing::Comment => self.comment.is_some(),
        }
    }

    /// Writes into an open text box, as typing would.
    #[cfg(test)]
    pub fn type_into(
        &mut self,
        editing: &Editing,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let (Editing::Title, Some((title, _))) = (editing, &self.title) {
            title.update(cx, |title, cx| title.set_value(text.to_owned(), window, cx));
        } else if let Some(state) = self.text_box(editing) {
            state.update(cx, |state, cx| state.set_value(text.to_owned(), window, cx));
        }
        cx.notify();
    }

    fn pull_request(&self, cx: &App) -> Option<PullRequest> {
        self.workspace
            .read(cx)
            .pull_requests
            .get(&self.thread_id)
            .cloned()
    }

    fn detail(&self, cx: &App) -> Option<PullRequestDetail> {
        self.workspace
            .read(cx)
            .pull_request_details
            .get(&self.thread_id)
            .cloned()
            .flatten()
    }

    /// Shows the cached detail at once; the engine reads a fresh one when it is old, and the
    /// files start loading now so the Code tab opens ready.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let version = self.pull_request(cx).as_ref().map(version);
        if version == self.version {
            cx.notify();
            return;
        }
        self.version = version;
        cx.notify();
        if self.version.is_none() {
            return;
        }
        if let Some(code) = &self.code {
            code.update(cx, |code, cx| code.reload(cx));
        }
        self.read_detail(cx);
    }

    fn read_detail(&self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        let thread_id = self.thread_id.clone();
        cx.spawn(async move |this, cx| {
            let read = thread_id.clone();
            let detail = cx
                .background_spawn(async move {
                    let detail = engine.pull_request_detail(&read);
                    engine.prefetch_pull_request(&read);
                    detail
                })
                .await;
            let Some(detail) = detail else {
                return;
            };
            this.update(cx, |this, cx| {
                this.workspace.update(cx, |workspace, cx| {
                    workspace
                        .pull_request_details
                        .insert(thread_id, Some(detail));
                    cx.notify();
                });
            })
            .ok();
        })
        .detach();
    }

    pub fn select(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == Tab::Code && self.code.is_none() {
            let engine = self.workspace.read(cx).engine.clone();
            let thread_id = self.thread_id.clone();
            let code =
                cx.new(|cx| ChangedFiles::new(engine, thread_id, Source::PullRequest, None, cx));
            cx.observe(&code, |_, _, cx| cx.notify()).detach();
            self.code = Some(code);
        }
        cx.notify();
    }

    /// Asks the forge for a change, then reads the detail again; `done` runs once it made it.
    fn run(
        &mut self,
        action: PullRequestAction,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.pending {
            return;
        }
        let forge_id = self
            .detail(cx)
            .map(|detail| detail.forge_id)
            .unwrap_or_default();
        self.pending = true;
        self.error = None;
        cx.notify();
        let forge = self.forge.clone();
        let thread_id = self.thread_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { forge.act(&thread_id, &forge_id, &action) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.read_detail(cx);
                        done(this, window, cx);
                    }
                    Err(error) => this.error = Some(forge_error_message(&error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn new_text_box(
        &self,
        initial: &str,
        placeholder: Option<String>,
        rows: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TextBox {
        let state = cx.new(|cx| {
            let state = TextareaState::new(window, cx)
                .auto_grow(rows.0, rows.1)
                .default_value(initial.to_owned());
            match placeholder {
                Some(placeholder) => state.placeholder(placeholder),
                None => state,
            }
        });
        let changes = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        TextBox {
            state,
            _changes: changes,
        }
    }

    fn text_box(&self, editing: &Editing) -> Option<&Entity<TextareaState>> {
        match editing {
            Editing::Title => None,
            Editing::Body => self.body.as_ref(),
            Editing::Reply(id) => self.replies.get(id),
            Editing::Comment => self.comment.as_ref(),
        }
        .map(|text_box| &text_box.state)
    }

    fn focused_editor(&self, window: &Window, cx: &App) -> Option<Editing> {
        if let Some((title, _)) = &self.title
            && title.focus_handle(cx).is_focused(window)
        {
            return Some(Editing::Title);
        }
        let mut boxes = self
            .body
            .iter()
            .map(|text_box| (Editing::Body, text_box))
            .chain(
                self.comment
                    .iter()
                    .map(|text_box| (Editing::Comment, text_box)),
            )
            .chain(
                self.replies
                    .iter()
                    .map(|(id, text_box)| (Editing::Reply(id.clone()), text_box)),
            );
        boxes
            .find(|(_, text_box)| text_box.state.focus_handle(cx).is_focused(window))
            .map(|(editing, _)| editing)
    }

    pub(in crate::panel) fn start_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pull_request) = self.pull_request(cx) else {
            return;
        };
        let state = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .default_value(pull_request.title.clone())
        });
        let events = cx.subscribe_in(&state, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.save(Editing::Title, window, cx);
            }
        });
        state.update(cx, |state, cx| state.focus(window, cx));
        self.title = Some((state, events));
        cx.notify();
    }

    fn start_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self
            .detail(cx)
            .map(|detail| detail.body)
            .unwrap_or_default();
        let text_box = self.new_text_box(&body, None, (6, 14), window, cx);
        text_box
            .state
            .update(cx, |state, cx| state.focus(window, cx));
        self.body = Some(text_box);
        cx.notify();
    }

    pub(in crate::panel) fn start_reply(
        &mut self,
        thread: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text_box = self.new_text_box("", Some(m::write_reply()), (3, 8), window, cx);
        text_box
            .state
            .update(cx, |state, cx| state.focus(window, cx));
        self.replies.insert(thread, text_box);
        cx.notify();
    }

    fn cancel(&mut self, editing: &Editing, cx: &mut Context<Self>) {
        match editing {
            Editing::Title => self.title = None,
            Editing::Body => self.body = None,
            Editing::Reply(id) => {
                self.replies.remove(id);
            }
            Editing::Comment => {}
        }
        cx.notify();
    }

    /// Sends what a text box holds; it closes, or empties for a comment, once the forge took it.
    pub(in crate::panel) fn save(
        &mut self,
        editing: Editing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        let (Some(pull_request), Some(detail)) = (self.pull_request(cx), self.detail(cx)) else {
            return;
        };
        if editing == Editing::Title {
            let Some((title, _)) = &self.title else {
                return;
            };
            let next = title.read(cx).value().trim().to_owned();
            if next.is_empty() || next == pull_request.title {
                self.cancel(&editing, cx);
                return;
            }
            let action = PullRequestAction::Edit {
                title: next,
                body: detail.body,
            };
            self.run(action, window, cx, |this, _, cx| {
                this.cancel(&Editing::Title, cx)
            });
            return;
        }
        let Some(text) = self
            .text_box(&editing)
            .map(|state| state.read(cx).value().to_string())
        else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        let action = match &editing {
            Editing::Body => PullRequestAction::Edit {
                title: pull_request.title,
                body: text,
            },
            Editing::Reply(id) => PullRequestAction::ReplyToThread {
                thread_id: id.clone(),
                body: text,
            },
            Editing::Comment => PullRequestAction::Comment { body: text },
            Editing::Title => return,
        };
        self.run(action, window, cx, move |this, window, cx| {
            if editing == Editing::Comment {
                if let Some(text_box) = &this.comment {
                    text_box
                        .state
                        .update(cx, |state, cx| state.set_value("", window, cx));
                }
            } else {
                this.cancel(&editing, cx);
            }
        });
    }

    fn address_comments(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        self.sending = true;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        let send = hoshi_core::Command::MessageSend {
            thread_id: self.thread_id.clone(),
            message_id: uuid::Uuid::new_v4().to_string(),
            text: prompt,
            attachments: Vec::new(),
        };
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_spawn(async move { engine.dispatch(send) })
                .await;
            if let Err(error) = sent {
                eprintln!("hoshi-native: could not address the comments: {error}");
            }
            this.update(cx, |this, cx| {
                this.sending = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn copy_checkout(&mut self, command: String, cx: &mut Context<Self>) {
        self.copy(command, cx);
        self.copied = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// A box with Save and, unless `cancels` is false, Cancel; Ctrl+Enter saves, Escape cancels.
    fn editor(
        &self,
        editing: Editing,
        save_label: String,
        cancels: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.text_box(&editing) else {
            return div().into_any_element();
        };
        let empty = state.read(cx).value().trim().is_empty();
        let key = editing.key();
        let cancel_editing = editing.clone();
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .debug_selector({
                        let key = key.clone();
                        move || format!("pr-editor-{key}")
                    })
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.input)
                    .px_2p5()
                    .py_1p5()
                    .text_xs()
                    .child(Textarea::new(state).appearance(false).xsmall()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_1p5()
                    .when(cancels, |row| {
                        row.child(
                            Button::new(SharedString::from(format!("{key}-cancel")))
                                .label(m::cancel())
                                .size(ButtonSize::Xs)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cancel(&cancel_editing, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("{key}-save")))
                            .label(save_label)
                            .size(ButtonSize::Xs)
                            .disabled(self.pending || empty)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save(editing.clone(), window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn error_banner(&self, colors: &Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let error = self.error.clone()?;
        Some(
            div()
                .debug_selector(|| "pr-error".into())
                .flex_none()
                .flex()
                .items_start()
                .gap_2()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.destructive.opacity(0.1))
                .text_xs()
                .text_color(colors.destructive)
                .child(div().flex_1().min_w_0().child(error))
                .child(
                    div()
                        .id("pr-error-dismiss")
                        .flex_none()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|dismiss| dismiss.text_color(colors.foreground))
                        .child(Icon::new(IconName::X).size_3p5())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        }))
                        .with_tooltip(m::dismiss()),
                )
                .into_any_element(),
        )
    }

    fn tabs(
        &self,
        detail: Option<&PullRequestDetail>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tabs = TABS
            .into_iter()
            .fold(Segmented::new("pr-tabs"), |tabs, tab| {
                tabs.segment(
                    Segment::new(
                        div()
                            .px_2p5()
                            .py_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tab.label()),
                    )
                    .selected(tab == self.tab)
                    .on_click(cx.listener(move |this, _, _, cx| this.select(tab, cx))),
                )
            });
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(colors.border)
            .child(tabs)
            .child(div().flex_1())
            .when_some(detail, |row, detail| {
                row.child(checks(&detail.checks, colors))
            })
    }

    fn code(&self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let Some(code) = &self.code else {
            return div().into_any_element();
        };
        let summary = match code.read(cx).files() {
            Files::Ready(files) if !files.is_empty() => Some((files.len(), totals(files))),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when_some(summary, |this, (count, (additions, deletions))| {
                this.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1p5()
                        .border_b_1()
                        .border_color(colors.border)
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::diff_file_count(count))
                        .child(DiffStat::new(additions, deletions)),
                )
            })
            .child(code.clone())
            .into_any_element()
    }
}

fn chip(colors: &Colors) -> Div {
    div()
        .h_6()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(colors.border)
}

fn section_title(text: String, colors: &Colors) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.muted_foreground)
        .child(text.to_uppercase())
}

fn markdown(id: &str, text: &str, colors: &Colors) -> TextView {
    crate::markdown::markdown(SharedString::from(id.to_owned()), text.to_owned(), colors)
}

fn card(colors: &Colors) -> Div {
    div()
        .rounded_lg()
        .border_1()
        .border_color(colors.border.opacity(0.6))
        .bg(colors.card.opacity(0.4))
        .text_xs()
}

fn checks(checks: &[PullRequestCheck], colors: &Colors) -> AnyElement {
    let Some((overall, label)) = checks_summary(checks) else {
        return div()
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(m::pr_no_checks())
            .into_any_element();
    };
    let (icon, color) = check_look(overall, colors);
    let (accent, muted) = (colors.accent, colors.foreground);
    let checks = checks.to_vec();
    Popover::new("pr-checks")
        .align(Align::End)
        .w(px(288.))
        .trigger(move |open| {
            div()
                .id("pr-checks-trigger")
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(open, |trigger| trigger.bg(accent))
                .hover(|trigger| trigger.bg(accent))
                .child(Icon::new(icon).size_3p5().text_color(color))
                .child(
                    div()
                        .text_color(if overall == ChecksState::Passing {
                            muted
                        } else {
                            color
                        })
                        .child(label),
                )
        })
        .content(move |_, _, cx| {
            let colors = cx.hoshi().clone();
            div()
                .id("pr-checks-list")
                .flex()
                .flex_col()
                .max_h(px(320.))
                .overflow_y_scroll()
                .p_1()
                .children(checks.into_iter().enumerate().map(|(index, check)| {
                    let (icon, color) = check_look(check.state, &colors);
                    let url = check.url.clone();
                    div()
                        .id(("pr-check", index))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .text_xs()
                        .when(url.is_some(), |row| {
                            row.cursor_pointer().hover(|row| row.bg(colors.accent))
                        })
                        .child(Icon::new(icon).size_3p5().flex_none().text_color(color))
                        .child(div().truncate().child(check.name))
                        .when_some(url, |row, url| {
                            row.on_click(move |_, _, cx| cx.open_url(&url))
                        })
                }))
        })
        .into_any_element()
}

/// Stands in for the detail on the first open, before anything is cached.
fn skeleton(colors: &Colors) -> Div {
    let bar = |width: DefiniteLength| div().h_3().w(width).rounded_sm().bg(colors.muted);
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .child(
            div()
                .flex()
                .gap_3()
                .child(bar(px(80.).into()))
                .child(bar(px(128.).into())),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(bar(px(80.).into()))
                .child(bar(px(96.).into())),
        )
        .child(div().mt_2().child(bar(px(96.).into())))
        .child(bar(relative(1.)))
        .child(bar(relative(0.92)))
        .child(bar(relative(0.8)))
        .child(bar(relative(0.66)))
}

impl Render for PullRequestSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let Some(pull_request) = workspace.pull_requests.get(&self.thread_id).cloned() else {
            return div()
                .flex_1()
                .child(panel_message(m::no_pull_request(), &colors))
                .into_any_element();
        };
        let read = workspace.pull_request_details.get(&self.thread_id).cloned();
        let detail = read.clone().flatten();
        let commenting = detail.is_some() && actions::can_comment(&pull_request);
        if commenting && self.tab == Tab::Timeline && self.comment.is_none() {
            self.comment =
                Some(self.new_text_box("", Some(m::write_comment()), (3, 8), window, cx));
        }
        let body = match (&detail, self.tab) {
            (None, _) => div()
                .child(if matches!(read, Some(None)) {
                    panel_message(m::pr_load_failed(), &colors)
                } else {
                    skeleton(&colors)
                })
                .into_any_element(),
            (Some(_), Tab::Code) => self.code(&colors, cx),
            (Some(detail), Tab::Summary) => self.summary(&pull_request, detail, &colors, cx),
            (Some(detail), Tab::Timeline) => self.timeline(detail, commenting, &colors, cx),
        };
        let scrolls = detail.is_none() || self.tab != Tab::Code;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.header(&pull_request, detail.as_ref(), &colors, cx))
            .children(self.error_banner(&colors, cx))
            .child(self.tabs(detail.as_ref(), &colors, cx))
            .child(if scrolls {
                div()
                    .id(("pr-body", self.tab as usize))
                    .debug_selector(|| "pr-body".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body)
                    .into_any_element()
            } else {
                body
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{checks_summary, format_review_threads, label_color};
    use hoshi_core::{ChecksState, PullRequestCheck, ReviewComment, ReviewThread};

    fn check(state: ChecksState) -> PullRequestCheck {
        PullRequestCheck {
            name: "ci".into(),
            state,
            url: None,
        }
    }

    #[test]
    fn failing_checks_win_over_running_ones() {
        assert_eq!(checks_summary(&[]), None);
        let mixed = [
            check(ChecksState::Passing),
            check(ChecksState::Pending),
            check(ChecksState::Failing),
        ];
        assert_eq!(
            checks_summary(&mixed).map(|(state, _)| state),
            Some(ChecksState::Failing)
        );
        let running = [check(ChecksState::Passing), check(ChecksState::Pending)];
        assert_eq!(
            checks_summary(&running).map(|(state, _)| state),
            Some(ChecksState::Pending)
        );
        let passing = [check(ChecksState::Passing)];
        assert_eq!(
            checks_summary(&passing).map(|(state, _)| state),
            Some(ChecksState::Passing)
        );
    }

    #[test]
    fn review_threads_read_as_where_then_who_said_what() {
        let comment = |author: Option<&str>, body: &str| ReviewComment {
            author: author.map(str::to_owned),
            body: body.into(),
            created_at: "2026-11-12T10:00:00Z".into(),
            url: "u".into(),
        };
        let open = ReviewThread {
            id: "1".into(),
            path: "src/a.rs".into(),
            line: Some(12),
            resolved: false,
            comments: vec![
                comment(Some("ana"), " Rename this\nplease "),
                comment(None, "ok"),
            ],
        };
        let moved = ReviewThread {
            id: "2".into(),
            path: "b.rs".into(),
            line: None,
            resolved: false,
            comments: vec![comment(Some("bo"), "Why?")],
        };
        let resolved = ReviewThread {
            resolved: true,
            ..moved.clone()
        };

        assert_eq!(
            format_review_threads(&[&open, &resolved, &moved]),
            "- src/a.rs:12\n  - ana: Rename this\n    please\n  - ?: ok\n- b.rs\n  - bo: Why?"
        );
    }

    #[test]
    fn label_colors_are_six_hex_digits() {
        assert!(label_color("d73a4a").is_some());
        assert!(label_color("#0e8a16").is_some());
        assert!(label_color("zzz").is_none());
        assert!(label_color("fff").is_none());
    }
}
