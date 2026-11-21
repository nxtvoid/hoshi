mod attachments;
pub(crate) use attachments::format_size;
pub(crate) use footer::{
    ACCESS_MODES, access_description, access_label, effective_effort, effort_label, fit_choice,
    model_options, workspace_option,
};
pub(crate) use model_picker::{
    ModelTab, PickerHost, PickerList, default_tab, provider_mark, provider_tint,
};
mod chips;
mod context;
mod delegated_request;
mod footer;
mod handoff;
mod held;
mod history;
mod model_picker;
mod notices;
mod planning_note;
mod queue;
mod request_panel;
mod suggestions;
mod trigger;

use crate::m;
use crate::requests::QuestionProgress;
use crate::state::Workspace;
use attachments::DraftAttachment;
use footer::FooterMenu;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use hoshi_core::{
    ApprovalDecision, Command, CommandError, ItemContent, MessageAttachment, SlashCommand,
    ThreadItem, ThreadStatus,
};
use hoshi_ui::prelude::*;
use hoshi_ui::prelude::{LanguageLabels, RichEditor, RichEditorEvent};
use request_panel::SecretField;
use std::collections::{HashMap, HashSet};
use suggestions::{Scope, Suggestion};
use trigger::Trigger;

/// One line of the docked editor, so the buttons beside it center on it.
const DOCKED_LINE: Pixels = px(32.);

const DROP_GROUP: &str = "composer-drop";

/// A command's failure as the composer says it.
pub(crate) fn command_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ThreadBusy { .. } => m::error_turn_running(),
        CommandError::BackgroundWorkRunning { .. } => m::error_background_work_running(),
        CommandError::QueuedMessageNotFound { .. } => m::queue_message_gone(),
        CommandError::ThreadNotFound { .. } => m::error_thread_not_found(),
        CommandError::ProjectNotFound { .. } => m::error_project_not_found(),
        CommandError::ThreadNotWorking { .. } => m::error_thread_not_working(),
        CommandError::EmptyMessage => m::error_empty_message(),
        CommandError::RequestNotPending { .. } => m::error_request_not_pending(),
        CommandError::RequestResponseInvalid { .. } => m::error_request_response_invalid(),
        CommandError::ProviderLocked { .. } => m::error_provider_locked(),
        CommandError::ThreadArchived { .. } => m::error_thread_archived(),
        CommandError::ThreadWaitingForUser { .. } => m::error_thread_waiting_for_user(),
        CommandError::MessageQueued { .. } => m::error_message_queued(),
        CommandError::NotAGitRepository { .. } => m::error_not_a_git_repository(),
        CommandError::WorktreeFailed { detail } => m::error_worktree_failed(detail),
        CommandError::WorkspaceRootNotFound { path } => m::error_workspace_root_not_found(path),
        CommandError::ChatNotLocal { .. } => m::error_chat_not_local(),
        CommandError::CannotRewind { .. } => m::error_cannot_rewind(),
        CommandError::NoCheckpoint { .. } => m::error_no_checkpoint(),
        CommandError::FolderUnavailable { path } => m::error_folder_unavailable(path),
        CommandError::GitFailed { detail } => m::error_git_failed(detail),
        _ => m::error_generic(),
    }
}

pub struct Composer {
    workspace: Entity<Workspace>,
    editor: Entity<RichEditor>,
    trigger: Option<Trigger>,
    /// The trigger Escape closed, kept closed until the token changes.
    dismissed: Option<Trigger>,
    suggestions: Vec<Suggestion>,
    /// Shown under the suggestions, or alone while there are none.
    status: Option<SharedString>,
    suggestion_scroll: ScrollHandle,
    editor_scroll: ScrollHandle,
    /// Where the caret was within the scrolled text at the last frame.
    caret_at: Option<Point<Pixels>>,
    reveal_caret: bool,
    active: usize,
    search: u64,
    commands: HashMap<Scope, Vec<SlashCommand>>,
    commands_loading: HashSet<Scope>,
    /// What the pull request and skill chips stand for, by chip id.
    mentions: HashMap<String, MessageAttachment>,
    attachments: Vec<DraftAttachment>,
    next_attachment: u64,
    error: Option<SharedString>,
    /// A passing note, such as where a large paste went.
    notice: Option<SharedString>,
    _notice_timer: Option<Task<()>>,
    placeholder: SharedString,
    /// How far the user got through each request's questions, by request id.
    progress: HashMap<String, QuestionProgress>,
    secret: Option<SecretField>,
    sending: bool,
    menu: Option<FooterMenu>,
    show_legacy: bool,
    model_search: Entity<InputState>,
    /// `None` until a rail button is pressed: the picker's default tab.
    model_tab: Option<ModelTab>,
    /// The docked editor outgrew one line; it stays wide until it is emptied.
    tall: bool,
    /// Past prompts, oldest first, for ↑.
    history: Vec<String>,
    recalled: Option<history::Recalled>,
    /// Where the text on screen is kept as a draft.
    draft_key: Option<String>,
    /// Images and files still being saved when the composer left their draft, by its key.
    parked: HashMap<String, Vec<DraftAttachment>>,
    /// Threads whose user chose to keep their full history on the next send.
    kept_history: HashSet<String>,
    notices: notices::Notices,
    _ticker: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Composer {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder: SharedString = m::composer_placeholder().into();
        let pasting = cx.weak_entity();
        let editor = cx.new(|cx| {
            let mut editor = RichEditor::new(window, cx);
            editor.set_placeholder(placeholder.clone(), window, cx);
            editor.set_language_labels(
                LanguageLabels {
                    plain: m::code_language_plain().into(),
                    none: m::code_language_none().into(),
                },
                m::code_language_search(),
                window,
                cx,
            );
            let opening = pasting.clone();
            editor.set_paste_handler(move |item, window, cx| {
                pasting
                    .update(cx, |this, cx| this.paste_attachments(item, window, cx))
                    .unwrap_or(false)
            });
            editor.set_token_click_handler(move |token, window, cx| {
                if let Some(key) = chips::attachment_key(token.id()) {
                    opening
                        .update(cx, |this, cx| this.open_attachment(key, window, cx))
                        .ok();
                }
            });
            editor
        });
        let this = cx.weak_entity();
        let menu_keys = cx.intercept_keystrokes(move |event, window, cx| {
            let handled = this
                .update(cx, |composer, cx| {
                    composer.menu_key(&event.keystroke, window, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let model_search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_models()));
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor),
            cx.subscribe_in(
                &model_search,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        this.pick_shortcut(0, window, cx);
                    }
                    _ => {}
                },
            ),
            menu_keys,
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.follow_draft(window, cx);
                cx.notify();
            }),
        ];
        cx.defer_in(window, |this: &mut Self, window, cx| {
            this.follow_draft(window, cx)
        });
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(context::CHECK_EVERY).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut composer = Self {
            workspace,
            editor,
            trigger: None,
            dismissed: None,
            suggestions: Vec::new(),
            status: None,
            suggestion_scroll: ScrollHandle::new(),
            editor_scroll: ScrollHandle::new(),
            caret_at: None,
            reveal_caret: false,
            active: 0,
            search: 0,
            commands: HashMap::new(),
            commands_loading: HashSet::new(),
            mentions: HashMap::new(),
            attachments: Vec::new(),
            next_attachment: 0,
            error: None,
            notice: None,
            _notice_timer: None,
            placeholder,
            progress: HashMap::new(),
            secret: None,
            sending: false,
            menu: FooterMenu::from_env(),
            show_legacy: false,
            model_search,
            model_tab: None,
            tall: false,
            history: Vec::new(),
            recalled: None,
            draft_key: None,
            parked: HashMap::new(),
            kept_history: HashSet::new(),
            notices: Default::default(),
            _ticker: ticker,
            _subscriptions: subscriptions,
        };
        composer.load_history(cx);
        composer
    }

    /// Keeps what is written as the draft of where it goes, then shows the draft of where the
    /// composer writes now: its text with its chips, and its saved images and files.
    fn follow_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.workspace.read(cx).draft_key(cx);
        if key == self.draft_key {
            return;
        }
        self.keep_draft(cx);
        let saving: Vec<DraftAttachment> = self
            .attachments
            .iter()
            .filter(|kept| kept.saved.is_none() && !kept.failed)
            .cloned()
            .collect();
        if let Some(left) = self.draft_key.clone()
            && !saving.is_empty()
        {
            self.parked.entry(left).or_default().extend(saving);
        }
        self.draft_key = key;
        let workspace = self.workspace.read(cx);
        let draft = self
            .draft_key
            .as_ref()
            .and_then(|key| workspace.drafts.get(key).cloned())
            .unwrap_or_default();
        let engine = workspace.engine.clone();
        self.attachments = draft
            .attachments
            .iter()
            .filter_map(|kept| {
                let path = engine.attachment_path(&kept.attachment)?;
                attachments::restored(kept, path.into())
            })
            .collect();
        if let Some(parked) = self
            .draft_key
            .as_ref()
            .and_then(|key| self.parked.remove(key))
        {
            self.attachments.extend(parked);
        }
        let highest = self.attachments.iter().map(|kept| kept.key).max();
        self.next_attachment = self.next_attachment.max(highest.unwrap_or(0));
        self.mentions = draft.mentions.iter().cloned().collect();
        self.error = None;
        let blocks = draft.snapshot();
        self.editor
            .update(cx, |editor, cx| editor.restore(blocks, window, cx));
    }

    /// Keeps what is written, as it is now, as the draft of where it goes.
    fn keep_draft(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.draft_key.clone() else {
            return;
        };
        let editor = self.editor.read(cx);
        let chips: HashSet<String> = editor
            .tokens(cx)
            .iter()
            .map(|span| span.token().id().to_string())
            .collect();
        let mut draft = crate::drafts::Draft::from_snapshot(editor.snapshot(cx));
        draft.attachments = self
            .attachments
            .iter()
            .filter_map(|kept| {
                Some(crate::drafts::Attachment {
                    key: kept.key,
                    chipped: kept.chipped,
                    attachment: kept.saved.clone()?,
                })
            })
            .collect();
        draft.mentions = self
            .mentions
            .iter()
            .filter(|(id, _)| chips.contains(*id))
            .map(|(id, mention)| (id.clone(), mention.clone()))
            .collect();
        draft.mentions.sort_by(|a, b| a.0.cmp(&b.0));
        self.workspace
            .update(cx, |workspace, cx| workspace.set_draft(&key, draft, cx));
    }

    /// Writes `draft` in place of what is written, as a message the thread went back before
    /// returns to be edited.
    pub fn restore_message(
        &mut self,
        draft: crate::drafts::Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.draft_key.take() {
            let draft = chips::chip_attachment_labels(draft);
            self.workspace
                .update(cx, |workspace, cx| workspace.set_draft(&key, draft, cx));
        }
        self.follow_draft(window, cx);
        self.focus(window, cx);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    /// Scrolls the text to the caret after it moves or the text changes, once laid out.
    fn caret_follower(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let editor = self.editor.clone();
        let scroll = self.editor_scroll.clone();
        canvas(
            move |_, _, cx| {
                let Some(caret) = editor.read(cx).caret_bounds(cx) else {
                    return;
                };
                let offset = scroll.offset();
                let at = point(caret.left(), caret.top() - offset.y);
                this.update(cx, |this, cx| {
                    let moved = this.caret_at != Some(at);
                    this.caret_at = Some(at);
                    if !(moved || std::mem::take(&mut this.reveal_caret)) {
                        return;
                    }
                    let view = scroll.bounds();
                    let margin = px(4.);
                    let mut y = offset.y;
                    if caret.bottom() + margin > view.bottom() {
                        y -= caret.bottom() + margin - view.bottom();
                    } else if caret.top() - margin < view.top() {
                        y += view.top() - (caret.top() - margin);
                    }
                    let y = y.clamp(-scroll.max_offset().y, px(0.));
                    if y != offset.y {
                        scroll.set_offset(point(offset.x, y));
                        cx.notify();
                    }
                })
                .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// Shows a note for a few seconds where errors show.
    pub(crate) fn show_notice(&mut self, notice: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(notice.into());
        self._notice_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(6))
                .await;
            this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The block the caret is in: where triggers are typed and chips go.
    fn input(&self, cx: &App) -> Option<Entity<TextareaState>> {
        self.editor.read(cx).active_input().cloned()
    }

    fn text(&self, cx: &App) -> String {
        self.editor.read(cx).markdown(cx)
    }

    fn set_text(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_markdown(text, window, cx));
    }

    #[cfg(test)]
    pub fn editor(&self) -> &Entity<RichEditor> {
        &self.editor
    }

    #[cfg(test)]
    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    fn on_editor(
        &mut self,
        _: &Entity<RichEditor>,
        event: &RichEditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RichEditorEvent::Submit => self.send(window, cx),
            RichEditorEvent::Change => {
                if self.editor.read(cx).is_empty(cx) {
                    self.tall = false;
                }
                self.reveal_caret = true;
                self.keep_draft(cx);
                self.update_trigger(cx);
                cx.notify();
            }
        }
    }

    /// Keys that steer the open menus, read before the editor sees them.
    fn menu_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.menu == Some(FooterMenu::Model) {
            return self.model_key(keystroke, window, cx);
        }
        let modifiers = &keystroke.modifiers;
        if keystroke.key == "v"
            && (modifiers.control || modifiers.platform)
            && modifiers.shift
            && !modifiers.alt
            && self.editor.read(cx).is_focused(window, cx)
        {
            return self.paste_as_text(window, cx);
        }
        let plain = !(modifiers.control || modifiers.alt || modifiers.platform);
        if plain && (self.suggestion_key(keystroke, window, cx) || self.request_key(keystroke, cx))
        {
            return true;
        }
        if self.history_key(keystroke, window, cx) {
            return true;
        }
        self.editor
            .update(cx, |editor, cx| editor.handle_key(keystroke, window, cx))
    }

    /// A digit picks an option of the question on screen while nothing is typed.
    fn request_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if keystroke.modifiers.shift || !self.editor.read(cx).is_empty(cx) {
            return false;
        }
        let Some(request) = self.pending_request(cx) else {
            return false;
        };
        let Some((label, multi_select)) = self.option_for_digit(&request, &keystroke.key) else {
            return false;
        };
        self.pick_option(&request, &label, multi_select, cx);
        true
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx).trim().to_owned();
        if let Some(request) = self.pending_request(cx) {
            self.answer_typed(&request, text, window, cx);
            return;
        }
        if self.attachments.is_empty()
            && let Some(action) = suggestions::standalone_action(&text, &self.actions(cx))
        {
            self.set_text("", window, cx);
            self.run_action(action, window, cx);
            return;
        }
        let ordered = self.ordered_attachments(cx);
        let failed = ordered.iter().any(|attachment| attachment.failed);
        if (text.is_empty() && ordered.is_empty()) || self.attachments_saving(cx) || failed {
            return;
        }
        let labels = chips::attachment_labels(
            &ordered
                .iter()
                .map(|attachment| (attachment.key, attachment.kind))
                .collect::<Vec<_>>(),
        );
        let raw = text;
        let text = chips::number_attachment_chips(&raw, &labels);
        let taken = std::mem::take(&mut self.attachments);
        let left_out: Vec<_> = taken
            .iter()
            .filter(|attachment| !ordered.iter().any(|sent| sent.key == attachment.key))
            .cloned()
            .collect();
        let attachments: Vec<MessageAttachment> = ordered
            .iter()
            .filter_map(|attachment| attachment.saved.clone())
            .chain(self.mentioned(cx))
            .collect();
        let compact_first = self.compacts_first(&raw, cx);
        let workspace = self.workspace.read(cx);
        let sending = match workspace.open_thread_id(cx) {
            Some(thread_id) => {
                let engine = workspace.engine.clone();
                let message_id = uuid::Uuid::new_v4().to_string();
                let text = text.clone();
                cx.background_spawn(async move {
                    if compact_first {
                        engine.compact_then_send(&thread_id, &message_id, text, attachments)
                    } else {
                        engine.dispatch(Command::MessageSend {
                            thread_id,
                            message_id,
                            text,
                            attachments,
                        })
                    }
                })
            }
            None if workspace.draft.is_some() => {
                let draft = text.clone();
                self.workspace.update(cx, |workspace, cx| {
                    workspace.send_draft(draft, attachments, cx)
                })
            }
            None => {
                self.attachments = taken;
                return;
            }
        };
        self.discard_all(&left_out, cx);
        self.set_text("", window, cx);
        self.recalled = None;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = sending.await;
            if result.is_ok() {
                this.update(cx, |this, cx| this.remember_prompt(&text, cx))
                    .ok();
            }
            if let Err(error) = result {
                this.update_in(cx, |this, window, cx| {
                    this.error = Some(command_error_message(&error).into());
                    this.set_text(&raw, window, cx);
                    let added = std::mem::take(&mut this.attachments);
                    this.attachments = ordered
                        .into_iter()
                        .map(chips::unchipped)
                        .chain(added)
                        .collect();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

impl Composer {
    /// Typed text answers the pending request: feedback that declines, or an own answer. A
    /// secret is typed in its own card, so it never reaches the chat.
    fn answer_typed(
        &mut self,
        request: &ThreadItem,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            return;
        }
        let answered = match &request.content {
            ItemContent::Questions { .. } => {
                let from = self.progress.get(&request.id).cloned().unwrap_or_default();
                self.advance_from(request, &from, &text, cx)
            }
            ItemContent::Approval { .. } | ItemContent::ProposedPlan { .. } => {
                self.decide(request, ApprovalDecision::Decline, Some(text), cx);
                true
            }
            _ => false,
        };
        if answered {
            self.set_text("", window, cx);
        }
    }

    fn interrupt(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let Some(thread_id) = workspace.open_thread_id(cx) else {
            return;
        };
        if workspace.stopping_since(&thread_id).is_some() {
            return;
        }
        let engine = workspace.engine.clone();
        self.workspace
            .update(cx, |workspace, cx| workspace.mark_stopping(&thread_id, cx));
        cx.spawn(async move |this, cx| {
            let stopped = thread_id.clone();
            let result = cx
                .background_spawn(async move {
                    engine.dispatch(Command::TurnInterrupt { thread_id: stopped })
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.workspace.update(cx, |workspace, cx| {
                        workspace.unmark_stopping(&thread_id, cx)
                    });
                    this.error = Some(command_error_message(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn stopping(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .open_thread_id(cx)
            .is_some_and(|id| workspace.stopping_since(&id).is_some())
    }

    fn working(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .open_thread_id(cx)
            .and_then(|id| workspace.model.thread(&id))
            .is_some_and(|thread| {
                matches!(
                    thread.status,
                    ThreadStatus::Working | ThreadStatus::WaitingForInput
                )
            })
    }

    fn sync_placeholder(
        &mut self,
        request: Option<&ThreadItem>,
        working: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder: SharedString = match request.map(|request| &request.content) {
            Some(ItemContent::SecretRequest { .. }) => m::secret_request_composer_placeholder(),
            Some(ItemContent::Approval { .. }) => m::request_approval_placeholder(),
            Some(ItemContent::Questions { .. }) => m::request_question_placeholder(),
            Some(ItemContent::ProposedPlan { .. }) => m::request_plan_placeholder(),
            _ if working => m::composer_steer_placeholder(),
            _ => m::composer_placeholder(),
        }
        .into();
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.editor.update(cx, |editor, cx| {
                editor.set_placeholder(placeholder, window, cx)
            });
        }
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let request = self.pending_request(cx);
        let delegated = request
            .is_none()
            .then(|| self.delegated_request(cx))
            .flatten();
        let working = self.working(cx);
        self.sync_placeholder(request.as_ref(), working, window, cx);
        let shown = request
            .as_ref()
            .or(delegated.as_ref().map(|(request, _)| request));
        if self
            .secret
            .as_ref()
            .is_some_and(|field| shown.is_none_or(|request| request.id != field.request_id()))
        {
            self.secret = None;
        }
        let panel = match (&request, &delegated) {
            (Some(request), _) => Some(self.render_request(request, None, window, cx)),
            (None, delegated) => {
                let delegated = delegated
                    .as_ref()
                    .map(|(request, task)| self.render_delegated(request, task, window, cx));
                let note = planning_note::planning_note(&self.workspace, cx);
                match (delegated, note) {
                    (Some(delegated), Some(note)) => Some(
                        div()
                            .flex()
                            .flex_col()
                            .child(delegated)
                            .child(note)
                            .into_any_element(),
                    ),
                    (delegated, note) => delegated.or(note),
                }
            }
        };
        let written = !self.text(cx).trim().is_empty();
        let secret_pending = matches!(
            request.as_ref().map(|request| &request.content),
            Some(ItemContent::SecretRequest { .. })
        );
        let attached = !self.attachments.is_empty();
        let blocked = self.attachments_saving(cx)
            || self.attached(cx).iter().any(|attachment| attachment.failed);
        let can_send = (written || (attached && request.is_none())) && !secret_pending && !blocked;
        let resumable =
            !working && request.is_none() && self.queue(cx).is_some_and(|queue| queue.can_resume);
        let (kind, action) = if working && !written && !attached {
            let stopping = self.stopping(cx);
            let stop = Button::new("stop")
                .icon(IconName::Square)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Secondary)
                .rounded_full()
                .disabled(stopping)
                .tooltip(
                    Tooltip::new(if stopping {
                        m::stopping()
                    } else {
                        m::composer_stop()
                    })
                    .side(Side::Top),
                )
                .on_click(cx.listener(|this, _, _, cx| this.interrupt(cx)));
            ("stop", stop)
        } else if resumable && !written && !attached {
            let resume = Button::new("resume")
                .icon(IconName::Play)
                .label(m::queue_resume())
                .size(ButtonSize::Sm)
                .rounded_full()
                .tooltip(Tooltip::new(m::queue_resume_hint()).side(Side::Top))
                .on_click(cx.listener(|this, _, _, cx| this.resume(cx)));
            ("resume", resume)
        } else {
            let send = Button::new("send")
                .icon(IconName::ArrowUp)
                .size(ButtonSize::IconSm)
                .rounded_full()
                .disabled(!can_send)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
            ("send", send)
        };
        let action = div()
            .debug_selector(move || format!("composer-{kind}"))
            .child(action);
        let footer = self.render_footer(cx);
        let notices = self.render_notices(cx);
        let held = self.render_held(cx);
        let queue = self.render_queue(cx);
        let handoff = self.render_handoff(cx);
        let suggestions = self.render_suggestions(cx);
        let tiles = self.render_attachments(cx);
        let attach = self.attach_button(cx);

        let hero = {
            let workspace = self.workspace.read(cx);
            workspace.open.is_none() && workspace.draft.is_some()
        };
        let infos = self.chip_infos();
        let colors = cx.hoshi().clone();
        self.editor.update(cx, |editor, _| {
            editor.set_token_renderer(move |context, _, _| {
                chips::render_chip(context, &infos, &colors)
            });
        });
        let stacked = hero || self.tall;
        let (inset_y, min_height) = if hero {
            (px(8.), px(64.))
        } else {
            (px(6.), DOCKED_LINE)
        };
        self.editor
            .update(cx, |editor, _| editor.set_inset(px(10.), inset_y));
        let measured = cx.weak_entity();
        let editor = div()
            .id("composer-editor")
            .relative()
            .flex_1()
            .min_w_0()
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx)))
            .child(
                div()
                    .id("composer-scroll")
                    .min_h(min_height)
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .track_scroll(&self.editor_scroll)
                    .child(self.editor.clone()),
            )
            .child(self.caret_follower(cx))
            .when(!stacked, |this| {
                this.child(
                    canvas(
                        move |bounds, _, cx| {
                            if bounds.size.height > DOCKED_LINE + px(8.) {
                                measured
                                    .update(cx, |this, cx| {
                                        this.tall = true;
                                        cx.notify();
                                    })
                                    .ok();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            });
        let body = if stacked {
            div().flex().flex_col().gap_1().child(editor).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(attach)
                    .child(action),
            )
        } else {
            div()
                .flex()
                .items_end()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(DOCKED_LINE)
                        .child(attach),
                )
                .child(editor)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(DOCKED_LINE)
                        .child(action),
                )
        };

        div()
            .id("composer")
            .group(DROP_GROUP)
            .pb_4()
            .pt_1()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.attach_paths(paths.paths().to_vec(), window, cx)
            }))
            .child(
                Lane::new()
                    .children(held)
                    .children(suggestions)
                    .children(notices)
                    .children(panel)
                    .children(queue)
                    .child(
                        InputFrame::new()
                            .roomy(hero)
                            .file_drop_group(DROP_GROUP)
                            .children(tiles)
                            .child(body)
                            .children(handoff),
                    )
                    .child(footer),
            )
    }
}

/// The composer against a real engine on a scratch home, with no provider.
#[cfg(test)]
mod ui_tests {
    use super::Composer;
    use crate::state::{self, Workspace};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
        WindowOptions, px, size,
    };
    use hoshi_core::{
        AccessMode, Command, Effort, Event, ItemContent, Question, QuestionOption, RequestStatus,
        ThreadItem, ThreadWorkspace,
    };
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Harness {
        cx: VisualTestContext,
        composer: Entity<Composer>,
        workspace: Entity<Workspace>,
        thread_id: String,
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("project/src")).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::write(dir.join("project/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("project/README.md"), "# readme").unwrap();
        dir
    }

    fn harness(cx: &mut TestAppContext, name: &str) -> Harness {
        cx.executor().allow_parking();
        let home = scratch(name);
        let (notify, notifications) = state::channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home.join("home"), notify).unwrap());
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "project".into(),
                workspace_root: home.join("project").to_string_lossy().into_owned(),
            })
            .unwrap();
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "t".into(),
                project_id: Some("p".into()),
                title: "thread".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
        workspace.update(cx, |workspace, cx| workspace.open_thread("t", cx));
        let (window, composer) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(400.)),
                })),
                ..Default::default()
            };
            let workspace = workspace.clone();
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Composer::new(workspace, window, cx))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        cx.run_until_parked();
        composer.update_in(&mut cx, |composer, window, cx| composer.focus(window, cx));
        cx.run_until_parked();
        Harness {
            cx,
            composer,
            workspace,
            thread_id: "t".into(),
        }
    }

    fn value(harness: &mut Harness) -> String {
        harness
            .composer
            .read_with(&harness.cx, |composer, cx| composer.text(cx))
    }

    #[gpui_kit::test]
    fn shift_enter_breaks_the_line_and_enter_sends(cx: &mut TestAppContext) {
        let mut h = harness(cx, "send");
        h.cx.simulate_input("first");
        h.cx.simulate_keystrokes("shift-enter");
        h.cx.simulate_input("second");
        assert_eq!(value(&mut h), "first\nsecond");

        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "");
        let sent = h.workspace.read_with(&h.cx, |workspace, cx| {
            workspace.open.as_ref().map(|open| {
                open.read(cx)
                    .items
                    .iter()
                    .filter_map(|item| match &item.content {
                        ItemContent::UserMessage { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        });
        assert_eq!(sent, Some(vec!["first\nsecond".to_owned()]));
        assert_eq!(h.thread_id, "t");
    }

    #[gpui_kit::test]
    fn typing_past_the_max_height_keeps_the_caret_in_view(cx: &mut TestAppContext) {
        let mut h = harness(cx, "follow");
        for line in 0..30 {
            if line > 0 {
                h.cx.simulate_keystrokes("shift-enter");
            }
            h.cx.simulate_input("line");
            h.cx.run_until_parked();
        }
        h.cx.update(|window, _| window.refresh());
        h.cx.run_until_parked();

        let (caret, view, offset) = h.composer.read_with(&h.cx, |composer, cx| {
            (
                composer.editor.read(cx).caret_bounds(cx).unwrap(),
                composer.editor_scroll.bounds(),
                composer.editor_scroll.offset(),
            )
        });
        assert!(offset.y < px(0.));
        assert!(caret.top() >= view.top() && caret.bottom() <= view.bottom());

        h.composer.read_with(&h.cx, |composer, _| {
            composer.editor_scroll.set_offset(Point::default())
        });
        h.cx.update(|window, _| window.refresh());
        h.cx.run_until_parked();
        let offset = h
            .composer
            .read_with(&h.cx, |composer, _| composer.editor_scroll.offset());
        assert_eq!(offset.y, px(0.));

        h.cx.simulate_keystrokes("left");
        h.cx.update(|window, _| window.refresh());
        h.cx.run_until_parked();
        let (caret, view) = h.composer.read_with(&h.cx, |composer, cx| {
            (
                composer.editor.read(cx).caret_bounds(cx).unwrap(),
                composer.editor_scroll.bounds(),
            )
        });
        assert!(caret.top() >= view.top() && caret.bottom() <= view.bottom());
    }

    #[gpui_kit::test]
    fn down_leaves_a_code_block_for_a_line_below(cx: &mut TestAppContext) {
        let mut h = harness(cx, "fence");
        h.cx.simulate_input("```tsx");
        h.cx.simulate_keystrokes("enter");
        h.cx.simulate_input("const a = 1");
        h.cx.run_until_parked();
        h.cx.simulate_keystrokes("down");
        h.cx.simulate_input("after");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "```tsx\nconst a = 1\n```\n\nafter");
    }

    #[gpui_kit::test]
    fn an_attached_file_is_saved_and_goes_with_the_message(cx: &mut TestAppContext) {
        let mut h = harness(cx, "attach");
        let source = std::env::temp_dir().join(format!("notes-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&source, "remember the milk").unwrap();
        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.attach_paths(vec![source], window, cx)
        });
        h.cx.run_until_parked();
        let saved = h.composer.read_with(&h.cx, |composer, _| {
            composer
                .attachments
                .first()
                .and_then(|attachment| attachment.saved.clone())
        });
        assert!(
            matches!(&saved, Some(hoshi_core::MessageAttachment::File { name, .. }) if name.starts_with("notes-")),
            "{saved:?}"
        );

        h.cx.simulate_input("read it");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        let left = h
            .composer
            .read_with(&h.cx, |composer, _| composer.attachments.len());
        assert_eq!(left, 0);
        let sent = h.workspace.read_with(&h.cx, |workspace, cx| {
            workspace.open.as_ref().and_then(|open| {
                open.read(cx)
                    .items
                    .iter()
                    .find_map(|item| match &item.content {
                        ItemContent::UserMessage { attachments, .. } => Some(attachments.clone()),
                        _ => None,
                    })
            })
        });
        assert_eq!(sent, saved.map(|saved| vec![saved]));
    }

    #[gpui_kit::test]
    fn a_threads_draft_keeps_its_chips_and_files_while_another_thread_is_open(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx, "draft-chips");
        let engine = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.engine.clone());
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "u".into(),
                project_id: Some("p".into()),
                title: "other".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        h.cx.simulate_input("read ");
        let source = std::env::temp_dir().join(format!("notes-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&source, "remember the milk").unwrap();
        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.attach_paths(vec![source], window, cx)
        });
        h.cx.run_until_parked();
        let shown = |h: &mut Harness| {
            h.composer.read_with(&h.cx, |composer, cx| {
                (
                    composer.text(cx),
                    composer.editor.read(cx).tokens(cx).len(),
                    composer.attachments.len(),
                )
            })
        };
        let before = shown(&mut h);
        assert_eq!((before.1, before.2), (1, 1), "{before:?}");

        h.workspace
            .update(&mut h.cx, |workspace, cx| workspace.open_thread("u", cx));
        h.cx.run_until_parked();
        assert_eq!(shown(&mut h), (String::new(), 0, 0));
        let kept = h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .drafts
                .get("t")
                .map(|draft| draft.attachments.len())
        });
        assert_eq!(kept, Some(1));

        h.workspace
            .update(&mut h.cx, |workspace, cx| workspace.open_thread("t", cx));
        h.cx.run_until_parked();
        assert_eq!(shown(&mut h), before);
    }

    #[gpui_kit::test]
    fn a_file_still_saving_when_the_thread_changes_joins_its_draft(cx: &mut TestAppContext) {
        let mut h = harness(cx, "draft-saving");
        let engine = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.engine.clone());
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "u".into(),
                project_id: Some("p".into()),
                title: "other".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        h.cx.simulate_input("read ");
        let source = std::env::temp_dir().join(format!("notes-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&source, "remember the milk").unwrap();
        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.attach_paths(vec![source], window, cx)
        });
        h.workspace
            .update(&mut h.cx, |workspace, cx| workspace.open_thread("u", cx));
        h.cx.run_until_parked();
        let kept = h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .drafts
                .get("t")
                .map(|draft| draft.attachments.len())
        });
        assert_eq!(kept, Some(1));

        h.workspace
            .update(&mut h.cx, |workspace, cx| workspace.open_thread("t", cx));
        h.cx.run_until_parked();
        let back = h.composer.read_with(&h.cx, |composer, _| {
            composer
                .attachments
                .iter()
                .filter(|attachment| attachment.saved.is_some())
                .count()
        });
        assert_eq!(back, 1);
    }

    #[gpui_kit::test]
    fn a_pasted_screenshot_is_attached_as_a_png_and_text_still_pastes(cx: &mut TestAppContext) {
        let mut h = harness(cx, "paste");
        let mut bmp = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4))
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .unwrap();
        let screenshot = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Bmp, bmp);
        h.cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&screenshot));
        h.cx.simulate_keystrokes("ctrl-v");
        h.cx.run_until_parked();

        let saved = h.composer.read_with(&h.cx, |composer, _| {
            composer
                .attachments
                .iter()
                .map(|attachment| attachment.saved.clone())
                .collect::<Vec<_>>()
        });
        assert!(
            matches!(
                saved.as_slice(),
                [Some(hoshi_core::MessageAttachment::Image { name, mime_type, .. })]
                    if name == "image.png" && mime_type == "image/png"
            ),
            "{saved:?}"
        );
        assert_eq!(value(&mut h), "");

        h.cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("plain".into()));
        h.cx.simulate_keystrokes("ctrl-v");
        h.cx.run_until_parked();
        assert_eq!(value(&mut h), "plain");
    }

    #[gpui_kit::test]
    fn a_large_paste_goes_as_a_file_unless_pasted_as_text(cx: &mut TestAppContext) {
        let mut h = harness(cx, "large-paste");
        let large = "x".repeat(super::attachments::PASTED_TEXT_ATTACHMENT_BYTES);
        h.cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(large.clone()));
        h.cx.simulate_keystrokes("ctrl-v");
        h.cx.run_until_parked();
        let names = h.composer.read_with(&h.cx, |composer, _| {
            composer
                .attachments
                .iter()
                .map(|attachment| attachment.name.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(names, ["pasted-text.txt"]);
        assert!(!value(&mut h).contains('x'));

        h.cx.simulate_keystrokes("ctrl-shift-v");
        h.cx.run_until_parked();
        assert!(value(&mut h).contains(&large));
    }

    #[gpui_kit::test]
    fn an_image_pasted_into_text_becomes_a_chip_sent_as_its_number(cx: &mut TestAppContext) {
        let mut h = harness(cx, "chip");
        h.cx.simulate_input("look at ");
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let shot = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, png);
        h.cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&shot));
        h.cx.simulate_keystrokes("ctrl-v");
        h.cx.run_until_parked();
        h.cx.simulate_input("please");
        assert_eq!(value(&mut h), "look at [image:1] please");

        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        let sent = h.workspace.read_with(&h.cx, |workspace, cx| {
            workspace.open.as_ref().and_then(|open| {
                open.read(cx)
                    .items
                    .iter()
                    .find_map(|item| match &item.content {
                        ItemContent::UserMessage {
                            text, attachments, ..
                        } => Some((text.clone(), attachments.len())),
                        _ => None,
                    })
            })
        });
        assert_eq!(sent, Some(("look at [Image #1] please".to_owned(), 1)));
    }

    #[gpui_kit::test]
    fn the_docked_editor_goes_above_its_buttons_past_one_line(cx: &mut TestAppContext) {
        let mut h = harness(cx, "tall");
        let tall = |h: &mut Harness| h.composer.read_with(&h.cx, |composer, _| composer.tall);
        h.cx.simulate_input("short");
        h.cx.run_until_parked();
        assert!(!tall(&mut h));

        h.cx.simulate_keystrokes("shift-enter");
        h.cx.simulate_input("second line");
        h.cx.run_until_parked();
        assert!(tall(&mut h));

        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.set_text("", window, cx)
        });
        h.cx.run_until_parked();
        assert!(!tall(&mut h));
    }

    #[gpui_kit::test]
    fn up_recalls_sent_prompts_and_down_steps_back_to_empty(cx: &mut TestAppContext) {
        let mut h = harness(cx, "history");
        h.cx.simulate_input("first");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();
        h.cx.simulate_input("second");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();
        assert_eq!(value(&mut h), "");

        h.cx.simulate_keystrokes("up");
        assert_eq!(value(&mut h), "second");
        h.cx.simulate_keystrokes("up");
        assert_eq!(value(&mut h), "first");
        h.cx.simulate_keystrokes("up");
        assert_eq!(value(&mut h), "first");
        h.cx.simulate_keystrokes("down");
        assert_eq!(value(&mut h), "second");
        h.cx.simulate_keystrokes("down");
        assert_eq!(value(&mut h), "");

        h.cx.simulate_input("typed");
        h.cx.simulate_keystrokes("up");
        assert_eq!(value(&mut h), "typed");
    }

    #[gpui_kit::test]
    fn a_slash_command_hoshi_runs_opens_the_model_picker(cx: &mut TestAppContext) {
        let mut h = harness(cx, "slash");
        h.cx.simulate_input("/mod");
        h.cx.run_until_parked();
        let first = h
            .composer
            .read_with(&h.cx, |composer, _| composer.suggestions().first().cloned());
        assert!(
            matches!(
                first,
                Some(super::Suggestion::Command {
                    action: Some(super::suggestions::ComposerAction::Model),
                    ..
                })
            ),
            "{first:?}"
        );

        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "");
        let open = h.composer.read_with(&h.cx, |composer, _| composer.menu);
        assert_eq!(open, Some(super::FooterMenu::Model));
    }

    #[gpui_kit::test]
    fn an_at_mention_becomes_a_chip_the_prompt_writes_as_a_path(cx: &mut TestAppContext) {
        let mut h = harness(cx, "mention");
        h.cx.simulate_input("read @mai");
        h.cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        h.cx.run_until_parked();

        let offered = h.composer.read_with(&h.cx, |composer, _| {
            composer
                .suggestions()
                .iter()
                .filter_map(|item| match item {
                    super::Suggestion::Path(entry) => Some(entry.path.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        assert!(offered.contains(&"src/main.rs".to_owned()), "{offered:?}");

        let index = offered
            .iter()
            .position(|path| path == "src/main.rs")
            .unwrap();
        for _ in 0..index {
            h.cx.simulate_keystrokes("down");
        }
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "read @src/main.rs");
        let editor = h
            .composer
            .read_with(&h.cx, |composer, _| composer.editor().clone());
        let tokens = editor.read_with(&h.cx, |editor, cx| {
            editor
                .tokens(cx)
                .iter()
                .map(|span| (span.range(), span.token().label().to_string()))
                .collect::<Vec<_>>()
        });
        assert_eq!(tokens, vec![(5..17, "main.rs".to_owned())]);

        h.cx.simulate_keystrokes("backspace");
        h.cx.simulate_keystrokes("backspace");
        assert_eq!(value(&mut h), "read");
    }

    /// Puts a request in the read model as if the agent had asked it. The engine never saw it,
    /// so answering it fails with `RequestNotPending`: that failure proves the answer went out.
    fn ask(h: &mut Harness, id: &str, content: ItemContent) {
        let item = ThreadItem {
            id: id.into(),
            thread_id: h.thread_id.clone(),
            created_at: "2026-11-12T10:00:00Z".into(),
            content,
        };
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::ItemAdded { item });
            cx.notify();
        });
        h.cx.run_until_parked();
    }

    fn state(h: &mut Harness) -> (String, Option<String>) {
        h.composer.read_with(&h.cx, |composer, _| {
            (
                composer.placeholder.to_string(),
                composer.error.as_ref().map(ToString::to_string),
            )
        })
    }

    fn question(labels: &[&str]) -> Question {
        Question {
            header: "Pick".into(),
            question: "Which one?".into(),
            options: labels
                .iter()
                .map(|label| QuestionOption {
                    label: (*label).into(),
                    description: String::new(),
                    preview: None,
                })
                .collect(),
            multi_select: false,
        }
    }

    #[gpui_kit::test]
    fn typed_text_declines_a_pending_approval(cx: &mut TestAppContext) {
        let mut h = harness(cx, "approval");
        ask(
            &mut h,
            "r1",
            ItemContent::Approval {
                tool_call_id: None,
                tool_name: "Bash".into(),
                input: r#"{"command":"rm -rf build"}"#.into(),
                reason: None,
                can_accept_for_session: true,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        );
        assert_eq!(state(&mut h).0, crate::m::request_approval_placeholder());

        h.cx.simulate_input("use cargo clean instead");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "");
        let error = state(&mut h).1.unwrap_or_default();
        assert_eq!(error, crate::m::error_request_not_pending());
    }

    #[gpui_kit::test]
    fn digits_answer_questions_one_by_one(cx: &mut TestAppContext) {
        let mut h = harness(cx, "questions");
        ask(
            &mut h,
            "q1",
            ItemContent::Questions {
                questions: vec![question(&["red", "blue"]), question(&["small", "big"])],
                status: RequestStatus::Pending,
                answers: None,
            },
        );
        assert_eq!(state(&mut h).0, crate::m::request_question_placeholder());

        h.cx.simulate_keystrokes("2");
        h.cx.run_until_parked();
        let progress = h
            .composer
            .read_with(&h.cx, |composer, _| composer.progress.get("q1").cloned());
        assert_eq!(
            progress.map(|progress| (progress.index, progress.answers)),
            Some((1, vec!["blue".to_owned()]))
        );
        assert_eq!(value(&mut h), "", "the digit picked, it was not typed");

        h.cx.simulate_keystrokes("1");
        h.cx.run_until_parked();
        let error = state(&mut h).1.unwrap_or_default();
        assert_eq!(error, crate::m::error_request_not_pending());
    }

    #[gpui_kit::test]
    fn a_long_question_wraps_inside_the_window(cx: &mut TestAppContext) {
        let mut h = harness(cx, "long-question");
        let long = Question {
            question: "Which of these should the note save? ".repeat(20),
            ..question(&["one", "two"])
        };
        ask(
            &mut h,
            "q1",
            ItemContent::Questions {
                questions: vec![long, question(&["red", "blue"])],
                status: RequestStatus::Pending,
                answers: None,
            },
        );
        h.cx.update(|window, cx| window.draw(cx).clear(cx));

        let text = h.cx.debug_bounds("question-text").unwrap();
        assert!(text.right() <= px(800.), "{text:?}");
        assert!(text.size.height > px(40.), "{text:?}");
    }

    #[gpui_kit::test]
    fn a_draft_becomes_a_thread_of_its_project_with_its_first_message(cx: &mut TestAppContext) {
        let mut h = harness(cx, "draft");
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.start_draft(Some("p".into()), cx)
        });
        h.composer.update(&mut h.cx, |composer, cx| {
            composer.set_access(AccessMode::Ask, false, cx)
        });
        h.cx.run_until_parked();
        h.cx.simulate_input("start here");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        let (thread, sent, draft) = h.workspace.read_with(&h.cx, |workspace, cx| {
            let open = workspace
                .open
                .as_ref()
                .expect("the new thread opens")
                .read(cx);
            let thread = workspace.model.thread(&open.thread_id).cloned().unwrap();
            let sent: Vec<String> = open
                .items
                .iter()
                .filter_map(|item| match &item.content {
                    ItemContent::UserMessage { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            (thread, sent, workspace.draft.clone())
        });
        assert_ne!(thread.id, "t");
        assert_eq!(thread.project_id.as_deref(), Some("p"));
        assert_eq!(thread.access_mode, AccessMode::Ask);
        assert_eq!(sent, vec!["start here".to_owned()]);
        assert_eq!(draft, None);
        assert_eq!(value(&mut h), "");
    }

    #[gpui_kit::test]
    fn a_delegated_tasks_request_shows_above_and_typing_still_goes_to_the_thread(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx, "delegated");
        let engine = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.engine.clone());
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "c".into(),
                project_id: Some("p".into()),
                title: "Fix CI".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        h.cx.run_until_parked();
        h.thread_id = "c".into();
        h.workspace.update(&mut h.cx, |workspace, _| {
            workspace.model.apply(&Event::ThreadDelegated {
                thread_id: "c".into(),
                parent_thread_id: "t".into(),
                shares_worktree: false,
            });
        });
        ask(
            &mut h,
            "r1",
            ItemContent::Approval {
                tool_call_id: None,
                tool_name: "Bash".into(),
                input: r#"{"command":"cargo test"}"#.into(),
                reason: None,
                can_accept_for_session: false,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        );
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(h.cx.debug_bounds("delegated-request").is_some());
        assert_ne!(state(&mut h).0, crate::m::request_approval_placeholder());

        h.cx.simulate_input("carry on");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();
        assert_eq!(user_messages(&mut h), vec!["carry on".to_owned()]);
        assert_eq!(state(&mut h).1, None);
    }

    #[gpui_kit::test]
    fn the_footer_sets_the_threads_access_and_effort(cx: &mut TestAppContext) {
        let mut h = harness(cx, "footer");
        h.composer.update(&mut h.cx, |composer, cx| {
            composer.set_access(AccessMode::Ask, true, cx);
            composer.set_model(|choice| choice.effort = Some(Effort::High), cx);
        });
        h.cx.run_until_parked();

        let thread = h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.thread("t").cloned().unwrap()
        });
        assert_eq!(thread.access_mode, AccessMode::Ask);
        assert!(thread.plan_mode);
        assert_eq!(thread.effort, Some(Effort::High));
    }

    #[gpui_kit::test]
    fn new_thread_choices_outlive_a_restart(cx: &mut TestAppContext) {
        let mut h = harness(cx, "choices");
        let home =
            std::env::temp_dir().join(format!("hoshi-native-choices-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        h.workspace
            .update(&mut h.cx, |workspace, _| workspace.load_drafts(&home));
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.start_draft(Some("p".into()), cx)
        });
        h.composer.update(&mut h.cx, |composer, cx| {
            composer.set_access(AccessMode::FullAccess, true, cx);
            composer.set_model(|choice| choice.fast_mode = true, cx);
        });
        h.cx.run_until_parked();

        let saved = crate::choices::load(&home);
        assert_eq!(saved.access_mode, AccessMode::FullAccess);
        assert!(saved.plan_mode);
        let kept = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.last_used.clone());
        assert_eq!(saved, kept);
    }

    #[gpui_kit::test]
    fn fixed_defaults_start_each_draft_and_a_change_in_one_stays_in_it(cx: &mut TestAppContext) {
        let mut h = harness(cx, "fixed-defaults");
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.settings.new_threads.access_mode = Some(AccessMode::FullAccess);
            workspace.settings.new_threads.workspace =
                Some(hoshi_core::NewThreadWorkspace::NewWorktree);
            workspace.start_draft(Some("p".into()), cx)
        });
        let access = |h: &mut Harness| {
            h.workspace.read_with(&h.cx, |workspace, _| {
                (
                    workspace.choices.access_mode,
                    workspace.last_used.access_mode,
                    workspace.choices.worktree,
                )
            })
        };
        assert_eq!(
            access(&mut h),
            (AccessMode::FullAccess, AccessMode::default(), true)
        );

        h.composer.update(&mut h.cx, |composer, cx| {
            composer.set_access(AccessMode::Ask, false, cx)
        });
        assert_eq!(access(&mut h), (AccessMode::Ask, AccessMode::Ask, true));

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.start_draft(Some("p".into()), cx)
        });
        assert_eq!(
            access(&mut h),
            (AccessMode::FullAccess, AccessMode::Ask, true)
        );

        h.workspace
            .update(&mut h.cx, |workspace, cx| workspace.start_draft(None, cx));
        assert!(!access(&mut h).2);
    }

    fn user_messages(h: &mut Harness) -> Vec<String> {
        h.workspace.read_with(&h.cx, |workspace, cx| {
            workspace
                .open
                .as_ref()
                .map(|open| {
                    open.read(cx)
                        .items
                        .iter()
                        .filter_map(|item| match &item.content {
                            ItemContent::UserMessage { text, .. } => Some(text.clone()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn stale_cache(h: &mut Harness) {
        h.cx.run_until_parked();
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.models = vec![hoshi_core::ProviderModels {
                provider: hoshi_core::default_provider(),
                compacts_idle_threads: true,
                ..Default::default()
            }];
            workspace.context_usages.insert(
                "t".into(),
                hoshi_core::ContextUsage {
                    used_tokens: 250_000,
                    max_tokens: Some(1_000_000),
                    cache_ttl_secs: Some(300),
                    cache_expires_at: Some(1),
                },
            );
            cx.notify();
        });
        h.cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_send_after_the_cache_ran_out_compacts_first(cx: &mut TestAppContext) {
        let mut h = harness(cx, "compact-first");
        stale_cache(&mut h);
        let due = h
            .composer
            .read_with(&h.cx, |composer, cx| composer.compaction(cx));
        assert_eq!(
            due,
            Some(super::context::Due {
                tokens: 250_000,
                compact: true
            })
        );

        h.cx.simulate_input("carry on");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(value(&mut h), "");
        assert_eq!(user_messages(&mut h), ["/compact", "carry on"]);
    }

    #[gpui_kit::test]
    fn a_held_message_shows_above_the_composer_and_edit_puts_it_back(cx: &mut TestAppContext) {
        let mut h = harness(cx, "held");
        let held = hoshi_core::HeldMessage {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: "carry on".into(),
            attachments: Vec::new(),
        };
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.held_messages.insert("t".into(), held.clone());
            cx.notify();
        });
        h.cx.run_until_parked();
        let shown = h
            .composer
            .update(&mut h.cx, |composer, cx| composer.render_held(cx).is_some());
        assert!(shown);

        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.fill(held, window, cx)
        });
        h.cx.run_until_parked();
        assert_eq!(value(&mut h), "carry on");

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.held_messages.clear();
            cx.notify();
        });
        let shown = h
            .composer
            .update(&mut h.cx, |composer, cx| composer.render_held(cx).is_some());
        assert!(!shown);
    }

    #[gpui_kit::test]
    fn no_chip_shows_when_idle_threads_never_compact_or_the_provider_does_not(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx, "compact-never");
        stale_cache(&mut h);
        let due = |h: &mut Harness| {
            h.composer
                .read_with(&h.cx, |composer, cx| composer.compaction(cx))
        };
        h.workspace.update(&mut h.cx, |workspace, _| {
            workspace.settings.providers.idle_compaction = hoshi_core::IdleCompaction::Never;
        });
        assert_eq!(due(&mut h), None);

        h.workspace.update(&mut h.cx, |workspace, _| {
            workspace.settings.providers.idle_compaction =
                hoshi_core::IdleCompaction::BeforeCacheExpires;
        });
        assert!(due(&mut h).is_some());

        h.workspace.update(&mut h.cx, |workspace, _| {
            workspace.models[0].compacts_idle_threads = false;
        });
        assert_eq!(due(&mut h), None);
    }

    #[gpui_kit::test]
    fn keeping_the_history_sends_without_compacting(cx: &mut TestAppContext) {
        let mut h = harness(cx, "keep-history");
        stale_cache(&mut h);
        h.composer.update(&mut h.cx, |composer, cx| {
            composer.kept_history.insert("t".into());
            cx.notify();
        });

        h.cx.simulate_input("carry on");
        h.cx.simulate_keystrokes("enter");
        h.cx.run_until_parked();

        assert_eq!(user_messages(&mut h), ["carry on"]);
    }

    fn pull_request(
        state: hoshi_core::PullRequestState,
        unresolved: u32,
    ) -> hoshi_core::PullRequest {
        hoshi_core::PullRequest {
            forge: hoshi_core::ForgeKind::Github,
            number: 42,
            title: "Login".into(),
            url: "https://github.com/nxtvoid/demo/pull/42".into(),
            author: None,
            head_branch: "feat/login".into(),
            head_sha: None,
            base_branch: "main".into(),
            state,
            checks: None,
            unresolved_comments: unresolved,
            open_tasks: 0,
            changes_requested: false,
            turn: None,
            conflicts: false,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-16T10:00:00Z".into(),
        }
    }

    /// Sets the thread's branch and its project root's, once the engine has told its own read
    /// of the root, which would land over them.
    fn on_branch(h: &mut Harness, branch: &str, root: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        h.cx.run_until_parked();
        while !h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.project_branches.contains_key("p")
        }) {
            assert!(
                std::time::Instant::now() < deadline,
                "the engine never read the root's branch"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
            h.cx.run_until_parked();
        }
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::ThreadBranchSet {
                thread_id: "t".into(),
                branch: Some(branch.into()),
            });
            workspace
                .project_branches
                .insert("p".into(), Some(root.into()));
            cx.notify();
        });
        redraw(h);
    }

    #[gpui_kit::test]
    fn the_branch_notice_waits_for_a_message_and_goes_once_the_root_is_back(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx, "branch-notice");
        on_branch(&mut h, "feat/login", "main");
        assert!(
            !shows(&mut h, "branch-notice"),
            "an idle thread is left alone"
        );

        h.cx.simulate_input("go on");
        redraw(&mut h);
        assert!(shows(&mut h, "branch-notice"));

        on_branch(&mut h, "feat/login", "feat/login");
        assert!(!shows(&mut h, "branch-notice"));
    }

    #[gpui_kit::test]
    fn open_review_comments_offer_their_address_while_the_agent_rests(cx: &mut TestAppContext) {
        let mut h = harness(cx, "review-notice");
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.pull_requests.insert(
                "t".into(),
                pull_request(hoshi_core::PullRequestState::Open, 2),
            );
            cx.notify();
        });
        redraw(&mut h);
        assert!(shows(&mut h, "review-notice"));

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::TurnStarted {
                thread_id: "t".into(),
                turn_id: "turn".into(),
                at: "2026-11-16T10:00:00Z".into(),
                model: None,
            });
            cx.notify();
        });
        redraw(&mut h);
        assert!(!shows(&mut h, "review-notice"), "not while the agent works");
    }

    #[gpui_kit::test]
    fn a_stop_holds_until_its_turn_ends(cx: &mut TestAppContext) {
        let mut h = harness(cx, "stopping");
        let since = |h: &mut Harness| {
            h.workspace
                .read_with(&h.cx, |workspace, _| workspace.stopping_since("t"))
        };
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::TurnStarted {
                thread_id: "t".into(),
                turn_id: "turn".into(),
                at: "2026-11-16T10:00:00Z".into(),
                model: None,
            });
            workspace.mark_stopping("t", cx);
        });
        assert!(since(&mut h).is_some());

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::TurnCompleted {
                thread_id: "t".into(),
                turn_id: "turn".into(),
                outcome: hoshi_core::TurnOutcome::Interrupted,
                at: "2026-11-16T10:00:05Z".into(),
            });
            cx.notify();
        });
        assert!(since(&mut h).is_none());
    }

    fn redraw(h: &mut Harness) {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn shows(h: &mut Harness, selector: &'static str) -> bool {
        h.cx.debug_bounds(selector).is_some()
    }

    fn queue(
        entries: &[(&str, hoshi_core::MessageSender)],
        can_resume: bool,
    ) -> hoshi_core::ThreadQueue {
        hoshi_core::ThreadQueue {
            thread_id: "t".into(),
            messages: entries
                .iter()
                .map(|(id, sender)| hoshi_core::QueueEntry {
                    message_id: (*id).into(),
                    text: format!("queued {id}"),
                    attachments: Vec::new(),
                    held: true,
                    sender: *sender,
                })
                .collect(),
            can_resume,
        }
    }

    #[gpui_kit::test]
    fn queued_messages_list_above_the_composer_and_an_empty_one_offers_resume(
        cx: &mut TestAppContext,
    ) {
        use hoshi_core::MessageSender::{Hoshi, User};
        let mut h = harness(cx, "queue");
        redraw(&mut h);
        assert!(!shows(&mut h, "queued-messages"));
        assert!(shows(&mut h, "composer-send"));

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace
                .queues
                .insert("t".into(), queue(&[("m1", User), ("m2", Hoshi)], true));
            cx.notify();
        });
        redraw(&mut h);
        assert!(shows(&mut h, "queued-messages"));
        assert!(shows(&mut h, "queue-send-m1"));
        assert!(
            !shows(&mut h, "queue-send-m2"),
            "hoshi's own news has no Send now"
        );
        assert!(shows(&mut h, "queue-remove-m2"));
        assert!(shows(&mut h, "composer-resume"));
        assert!(!shows(&mut h, "composer-send"));

        h.cx.simulate_input("more");
        redraw(&mut h);
        assert!(shows(&mut h, "composer-send"), "written text sends");
        h.composer.update_in(&mut h.cx, |composer, window, cx| {
            composer.set_text("", window, cx)
        });

        let remove = h.cx.debug_bounds("queue-remove-m1").unwrap().center();
        h.cx.simulate_click(remove, gpui_kit::Modifiers::none());
        h.cx.run_until_parked();
        assert_eq!(state(&mut h).1, Some(crate::m::queue_message_gone()));
    }

    #[gpui_kit::test]
    fn send_now_waits_for_the_running_turn_and_resume_for_an_idle_thread(cx: &mut TestAppContext) {
        let mut h = harness(cx, "queue-busy");
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.queues.insert(
                "t".into(),
                queue(&[("m1", hoshi_core::MessageSender::User)], true),
            );
            workspace.model.apply(&Event::TurnStarted {
                thread_id: "t".into(),
                turn_id: "turn".into(),
                at: "2026-11-15T10:00:00Z".into(),
                model: None,
            });
            cx.notify();
        });
        redraw(&mut h);
        assert!(shows(&mut h, "queued-messages"));
        assert!(!shows(&mut h, "queue-send-m1"));
        assert!(!shows(&mut h, "composer-resume"));
    }

    #[gpui_kit::test]
    fn picking_a_permission_closes_its_menu(cx: &mut TestAppContext) {
        let mut h = harness(cx, "access-menu");
        h.composer.update(&mut h.cx, |composer, cx| {
            composer.set_menu(super::footer::FooterMenu::Access, true, cx)
        });
        redraw(&mut h);
        assert!(shows(&mut h, "menu-item-access-FullAccess"));
        h.cx.simulate_keystrokes("end enter");
        redraw(&mut h);
        let access = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.choices.access_mode);
        assert_eq!(access, hoshi_core::AccessMode::FullAccess, "the pick took");
        let menu = h.composer.read_with(&h.cx, |composer, _| composer.menu);
        assert_eq!(menu, None);
    }

    #[gpui_kit::test]
    fn a_pending_provider_switch_shows_in_the_composer_until_cancelled(cx: &mut TestAppContext) {
        let mut h = harness(cx, "handoff");
        redraw(&mut h);
        assert!(!shows(&mut h, "handoff-pending"));

        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.model.apply(&Event::ThreadModelSet {
                thread_id: "t".into(),
                provider: hoshi_core::default_provider(),
                model: Some("sonnet".into()),
                effort: None,
                fast_mode: false,
                ultracode: false,
            });
            workspace.model.apply(&Event::ThreadModelPending {
                thread_id: "t".into(),
                choice: Some(hoshi_core::ModelChoice {
                    provider: "codex".into(),
                    model: Some("gpt".into()),
                    effort: None,
                    fast_mode: false,
                    ultracode: false,
                }),
            });
            cx.notify();
        });
        redraw(&mut h);
        assert!(shows(&mut h, "handoff-pending"));
        let text = h.workspace.read_with(&h.cx, |workspace, _| {
            crate::m::handoff_pending(
                super::handoff::model_name(
                    workspace,
                    &hoshi_core::default_provider(),
                    Some("sonnet"),
                ),
                super::handoff::model_name(workspace, "codex", Some("gpt")),
            )
        });
        assert_eq!(text, "Context handoff pending · sonnet → gpt");
        let footer = h.composer.read_with(&h.cx, |composer, cx| {
            composer.target(cx).map(|target| target.choice.provider)
        });
        assert_eq!(
            footer.as_deref(),
            Some("codex"),
            "the footer shows the model the next message uses"
        );

        let cancel = h.cx.debug_bounds("handoff-cancel").unwrap().center();
        h.cx.simulate_click(cancel, gpui_kit::Modifiers::none());
        h.cx.run_until_parked();
        let engine = h
            .workspace
            .read_with(&h.cx, |workspace, _| workspace.engine.clone());
        let kept = engine
            .snapshot()
            .threads
            .into_iter()
            .find(|thread| thread.id == "t")
            .and_then(|thread| thread.model);
        assert_eq!(
            kept.as_deref(),
            Some("sonnet"),
            "the current model went back"
        );
    }
}
