//! The `@`, `/`, `#` and `$` menu above the composer: files and folders, commands, pull
//! requests and skills, and what picking one writes in the prompt.

use super::Composer;
use super::trigger::{Trigger, TriggerKind, detect_trigger, rank_commands};
use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::component::input::InlineToken;
use gpui_kit::*;
use hoshi_core::{
    CommandError, EntryKind, MessageAttachment, PullRequest, PullRequestState, SlashCommand,
    WorkspaceEntry,
};
use hoshi_ui::prelude::*;
use std::ops::Range;
use std::time::Duration;

const PATH_SEARCH_DELAY: Duration = Duration::from_millis(50);
const PULL_REQUEST_SEARCH_DELAY: Duration = Duration::from_millis(200);

/// Commands hoshi runs itself instead of sending them to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerAction {
    Model,
    Plan,
    Default,
}

impl ComposerAction {
    fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Plan => "plan",
            Self::Default => "default",
        }
    }

    fn command(self) -> SlashCommand {
        SlashCommand {
            name: self.name().into(),
            description: match self {
                Self::Model => m::composer_command_model(),
                Self::Plan => m::composer_command_plan(),
                Self::Default => m::composer_command_default(),
            },
            argument_hint: None,
            aliases: Vec::new(),
            custom: false,
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Model => IconName::Brain,
            Self::Plan => IconName::ListTodo,
            Self::Default => IconName::Pencil,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Suggestion {
    Path(WorkspaceEntry),
    Command {
        command: SlashCommand,
        action: Option<ComposerAction>,
    },
    PullRequest(PullRequest),
    Skill(SlashCommand),
}

/// The commands and skills a prompt can start with, offered for a `/` or `$` typed.
pub fn command_suggestions(
    kind: TriggerKind,
    commands: &[SlashCommand],
    actions: &[ComposerAction],
    query: &str,
) -> Vec<Suggestion> {
    match kind {
        TriggerKind::Skill => {
            let skills = commands
                .iter()
                .filter(|command| command.custom)
                .cloned()
                .collect();
            rank_commands(skills, query, |command| command)
                .into_iter()
                .map(Suggestion::Skill)
                .collect()
        }
        _ => {
            let own = actions
                .iter()
                .map(|action| (action.command(), Some(*action)));
            let provider = commands
                .iter()
                .filter(|command| !actions.iter().any(|action| action.name() == command.name))
                .map(|command| (command.clone(), None));
            rank_commands(own.chain(provider).collect(), query, |(command, _)| command)
                .into_iter()
                .map(|(command, action)| Suggestion::Command { command, action })
                .collect()
        }
    }
}

/// What a lone `/model`, `/plan` or `/default` asks for, when hoshi runs it.
pub fn standalone_action(text: &str, actions: &[ComposerAction]) -> Option<ComposerAction> {
    let name = text.trim().strip_prefix('/')?;
    actions.iter().copied().find(|action| action.name() == name)
}

/// How the prompt writes a file mention.
pub fn mention_source(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@\"{path}\"")
    } else {
        format!("@{path}")
    }
}

fn file_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// Where the composer's searches look: the open thread, or the draft's project.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Scope {
    pub project_id: Option<String>,
    pub thread_id: Option<String>,
}

impl Composer {
    pub(super) fn scope(&self, cx: &App) -> Scope {
        let workspace = self.workspace.read(cx);
        match workspace.open_thread_id(cx) {
            Some(thread_id) => Scope {
                project_id: workspace
                    .model
                    .thread(&thread_id)
                    .and_then(|thread| thread.project_id.clone()),
                thread_id: Some(thread_id),
            },
            None => Scope {
                project_id: workspace
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.project_id.clone()),
                thread_id: None,
            },
        }
    }

    pub(super) fn actions(&self, cx: &App) -> Vec<ComposerAction> {
        let plan_enabled = self.workspace.read(cx).settings.plan_mode_enabled;
        let plan_mode = self.target(cx).is_some_and(|target| target.plan_mode);
        let mut actions = vec![ComposerAction::Model];
        if plan_enabled && !plan_mode {
            actions.push(ComposerAction::Plan);
        }
        if plan_mode {
            actions.push(ComposerAction::Default);
        }
        actions
    }

    pub(super) fn run_action(
        &mut self,
        action: ComposerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ComposerAction::Model => self.open_model_menu(true, window, cx),
            ComposerAction::Plan | ComposerAction::Default => {
                if let Some(target) = self.target(cx) {
                    self.set_access(target.access_mode, action == ComposerAction::Plan, cx);
                }
            }
        }
    }

    pub(super) fn update_trigger(&mut self, cx: &mut Context<Self>) {
        let rich = self.editor.read(cx);
        let in_code = rich.in_code_block();
        let first_block = rich.in_first_block();
        let trigger = rich
            .active_input()
            .and_then(|input| {
                let editor = input.read(cx);
                let tokens: Vec<Range<usize>> =
                    editor.tokens().iter().map(|span| span.range()).collect();
                detect_trigger(&editor.value(), editor.cursor(), &tokens)
            })
            .filter(|trigger| !in_code && (first_block || trigger.kind != TriggerKind::Command))
            .filter(|trigger| self.dismissed.as_ref() != Some(trigger));
        if trigger == self.trigger {
            return;
        }
        let same_kind = trigger.as_ref().map(|trigger| trigger.kind)
            == self.trigger.as_ref().map(|trigger| trigger.kind);
        self.trigger = trigger.clone();
        self.dismissed = None;
        self.active = 0;
        self.search += 1;
        if !same_kind {
            self.suggestions.clear();
        }
        let Some(trigger) = trigger else {
            self.status = None;
            cx.notify();
            return;
        };
        match trigger.kind {
            TriggerKind::Path => self.search_paths(trigger, cx),
            TriggerKind::PullRequest => self.search_pull_requests(trigger, cx),
            TriggerKind::Command | TriggerKind::Skill => {
                self.load_commands(cx);
                self.offer_commands(cx);
            }
        }
        cx.notify();
    }

    fn searching(&mut self, message: String) {
        if self.suggestions.is_empty() {
            self.status = Some(message.into());
        }
    }

    fn search_paths(&mut self, trigger: Trigger, cx: &mut Context<Self>) {
        self.searching(m::composer_suggestions_searching_files());
        let generation = self.search;
        let engine = self.workspace.read(cx).engine.clone();
        let scope = self.scope(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PATH_SEARCH_DELAY).await;
            if this
                .update(cx, |this, _| this.search != generation)
                .unwrap_or(true)
            {
                return;
            }
            let found = cx
                .background_spawn(async move {
                    engine.search_paths(
                        scope.project_id.as_deref(),
                        scope.thread_id.as_deref(),
                        &trigger.query,
                    )
                })
                .await
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                if this.search == generation {
                    this.status = found
                        .is_empty()
                        .then(|| m::composer_suggestions_no_files().into());
                    this.suggestions = found.into_iter().map(Suggestion::Path).collect();
                    this.active = 0;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn search_pull_requests(&mut self, trigger: Trigger, cx: &mut Context<Self>) {
        let Some(project_id) = self.scope(cx).project_id else {
            self.suggestions.clear();
            self.status = Some(m::composer_suggestions_pull_requests_unavailable().into());
            return;
        };
        self.searching(m::composer_suggestions_searching_pull_requests());
        let generation = self.search;
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(PULL_REQUEST_SEARCH_DELAY)
                .await;
            if this
                .update(cx, |this, _| this.search != generation)
                .unwrap_or(true)
            {
                return;
            }
            let found = cx
                .background_spawn(async move {
                    engine.search_pull_requests(&project_id, &trigger.query)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.search != generation {
                    return;
                }
                match found {
                    Ok(found) => {
                        this.status = found
                            .is_empty()
                            .then(|| m::composer_suggestions_no_pull_requests().into());
                        this.suggestions = found.into_iter().map(Suggestion::PullRequest).collect();
                    }
                    Err(error) => {
                        if !matches!(error, CommandError::ForgeUnavailable) {
                            eprintln!("hoshi: pull request search failed: {error}");
                        }
                        this.suggestions.clear();
                        this.status =
                            Some(m::composer_suggestions_pull_requests_unavailable().into());
                    }
                }
                this.active = 0;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks the provider for its commands once per scope; they can take a moment.
    fn load_commands(&mut self, cx: &mut Context<Self>) {
        let scope = self.scope(cx);
        if self.commands.contains_key(&scope) || !self.commands_loading.insert(scope.clone()) {
            return;
        }
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let wanted = scope.clone();
            let found = cx
                .background_spawn(async move {
                    engine.slash_commands(wanted.project_id.as_deref(), wanted.thread_id.as_deref())
                })
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                this.commands_loading.remove(&scope);
                this.commands.insert(scope, found);
                this.offer_commands(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn offer_commands(&mut self, cx: &mut Context<Self>) {
        let Some(trigger) = self
            .trigger
            .clone()
            .filter(|trigger| matches!(trigger.kind, TriggerKind::Command | TriggerKind::Skill))
        else {
            return;
        };
        let scope = self.scope(cx);
        let loaded = self.commands.get(&scope);
        let actions = self.actions(cx);
        self.suggestions = command_suggestions(
            trigger.kind,
            loaded.map(Vec::as_slice).unwrap_or_default(),
            &actions,
            &trigger.query,
        );
        self.active = self.active.min(self.suggestions.len().saturating_sub(1));
        self.status = if loaded.is_none() && self.commands_loading.contains(&scope) {
            Some(m::composer_suggestions_loading_commands().into())
        } else if self.suggestions.is_empty() {
            Some(
                match trigger.kind {
                    TriggerKind::Skill => m::composer_suggestions_no_skills(),
                    _ => m::composer_suggestions_no_commands(),
                }
                .into(),
            )
        } else {
            None
        };
    }

    /// Escape closes the menu until the token being typed changes.
    pub(super) fn dismiss_suggestions(&mut self, cx: &mut Context<Self>) {
        self.dismissed = self.trigger.take();
        self.suggestions.clear();
        self.status = None;
        self.search += 1;
        cx.notify();
    }

    pub(super) fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.suggestions.len() as isize;
        if count > 0 {
            self.active = (self.active as isize + delta).rem_euclid(count) as usize;
            self.suggestion_scroll.scroll_to_item(self.active);
            cx.notify();
        }
    }

    fn replace_with_token(
        &mut self,
        range: Range<usize>,
        token: InlineToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.input(cx) else {
            return;
        };
        input.update(cx, |editor, cx| {
            if editor
                .replace_range_with_token(range, token, window, cx)
                .is_ok()
            {
                editor.insert(" ", window, cx);
            }
        });
    }

    fn replace_with_text(
        &mut self,
        range: Range<usize>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.input(cx) else {
            return;
        };
        input.update(cx, |editor, cx| {
            editor.set_selected_range(range, cx);
            editor.replace(text, window, cx);
        });
    }

    pub(super) fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(trigger), Some(item)) =
            (self.trigger.clone(), self.suggestions.get(index).cloned())
        else {
            return;
        };
        self.trigger = None;
        self.suggestions.clear();
        self.status = None;
        self.search += 1;
        let range = trigger.range.clone();
        match item {
            Suggestion::Path(entry) => {
                let directory = entry.kind == EntryKind::Directory;
                if directory && !entry.path.chars().any(char::is_whitespace) {
                    self.replace_with_text(range, format!("@{}/", entry.path), window, cx);
                } else {
                    let token = InlineToken::new(
                        format!("file:{}", entry.path),
                        mention_source(&entry.path),
                    )
                    .with_label(file_name(&entry.path));
                    self.replace_with_token(range, token, window, cx);
                }
            }
            Suggestion::PullRequest(pull_request) => {
                let id = format!("pr:{}", pull_request.number);
                let source = format!("#{}", pull_request.number);
                self.mentions.insert(
                    id.clone(),
                    MessageAttachment::PullRequest {
                        number: pull_request.number,
                        title: pull_request.title,
                        url: pull_request.url,
                        head_branch: pull_request.head_branch,
                        base_branch: pull_request.base_branch,
                        state: pull_request.state,
                    },
                );
                self.replace_with_token(range, InlineToken::new(id, source), window, cx);
            }
            Suggestion::Skill(command) => {
                let id = format!("skill:{}", command.name);
                let source = format!("${}", command.name);
                self.mentions
                    .insert(id.clone(), MessageAttachment::Skill { name: command.name });
                self.replace_with_token(range, InlineToken::new(id, source), window, cx);
            }
            Suggestion::Command {
                action: Some(action),
                ..
            } => {
                self.replace_with_text(range, String::new(), window, cx);
                self.run_action(action, window, cx);
            }
            Suggestion::Command { command, .. } => {
                self.replace_with_text(range, format!("/{} ", command.name), window, cx);
            }
        }
        self.update_trigger(cx);
        cx.notify();
    }

    /// The pull requests and skills the prompt's chips mention, in order, each once.
    pub(super) fn mentioned(&self, cx: &App) -> Vec<MessageAttachment> {
        let mut found: Vec<MessageAttachment> = Vec::new();
        for span in self.editor.read(cx).tokens(cx) {
            if let Some(attachment) = self.mentions.get(span.token().id().as_ref())
                && !found.contains(attachment)
            {
                found.push(attachment.clone());
            }
        }
        found
    }

    /// Keys that steer the open menu, read before the editor sees them.
    pub(super) fn suggestion_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.trigger.is_none() {
            return false;
        }
        match keystroke.key.as_str() {
            "up" if !self.suggestions.is_empty() => self.move_active(-1, cx),
            "down" if !self.suggestions.is_empty() => self.move_active(1, cx),
            "enter" | "tab" if !keystroke.modifiers.shift && !self.suggestions.is_empty() => {
                self.pick(self.active, window, cx)
            }
            "escape" => self.dismiss_suggestions(cx),
            _ => return false,
        }
        true
    }

    pub(super) fn render_suggestions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let trigger = self.trigger.as_ref()?;
        if self.suggestions.is_empty() && self.status.is_none() {
            return None;
        }
        let colors = cx.hoshi().clone();
        let heading = match trigger.kind {
            TriggerKind::Path => m::composer_suggestions_files(),
            TriggerKind::Command => m::composer_suggestions_commands(),
            TriggerKind::PullRequest => m::composer_suggestions_pull_requests(),
            TriggerKind::Skill => m::composer_suggestions_skills(),
        };
        let items = self.suggestions.iter().enumerate().map(|(index, item)| {
            let (icon, label, hint, detail) = describe(item, &colors);
            Choice::new(("suggestion", index), label)
                .icon(icon)
                .hint(hint)
                .detail(detail)
                .active(index == self.active)
                .on_click(cx.listener(move |this, _, window, cx| this.pick(index, window, cx)))
        });
        Some(
            ChoiceMenu::new("suggestions", items.collect::<Vec<_>>())
                .heading(heading)
                .status(self.status.clone())
                .track_scroll(&self.suggestion_scroll)
                .into_any_element(),
        )
    }
}

fn describe(item: &Suggestion, colors: &Colors) -> (AnyElement, String, Option<String>, String) {
    let icon = |name: IconName| Icon::new(name).size_4().into_any_element();
    match item {
        Suggestion::Path(entry) => {
            let directory = entry.kind == EntryKind::Directory;
            let (parent, name) = match entry.path.rfind('/') {
                Some(slash) => (&entry.path[..slash], &entry.path[slash + 1..]),
                None => ("", entry.path.as_str()),
            };
            (
                icon(if directory {
                    IconName::Folder
                } else {
                    IconName::File
                }),
                format!("{name}{}", if directory { "/" } else { "" }),
                None,
                parent.to_owned(),
            )
        }
        Suggestion::PullRequest(pull_request) => {
            let (name, color) = match pull_request.state {
                PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open),
                PullRequestState::Draft => (IconName::GitPullRequestDraft, colors.muted_foreground),
                PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged),
                PullRequestState::Closed => (IconName::GitPullRequestClosed, colors.destructive),
            };
            (
                Icon::new(name)
                    .size_4()
                    .text_color(color)
                    .into_any_element(),
                format!("#{}", pull_request.number),
                Some(pull_request.head_branch.clone()),
                pull_request.title.clone(),
            )
        }
        Suggestion::Skill(command) => (
            icon(IconName::Sparkles),
            format!("${}", command.name),
            None,
            command.description.clone(),
        ),
        Suggestion::Command { command, action } => (
            icon(action.map_or(IconName::Slash, ComposerAction::icon)),
            format!("/{}", command.name),
            command.argument_hint.clone(),
            command.description.clone(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ComposerAction, Suggestion, command_suggestions, mention_source, standalone_action,
    };
    use crate::composer::trigger::TriggerKind;
    use hoshi_core::SlashCommand;

    fn command(name: &str, custom: bool) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: String::new(),
            argument_hint: None,
            aliases: Vec::new(),
            custom,
        }
    }

    fn names(found: &[Suggestion]) -> Vec<String> {
        found
            .iter()
            .map(|item| match item {
                Suggestion::Command { command, .. } | Suggestion::Skill(command) => {
                    command.name.clone()
                }
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn hoshi_runs_its_own_commands_ahead_of_the_providers() {
        let commands = [
            command("model", false),
            command("review", false),
            command("deploy", true),
        ];
        let actions = [ComposerAction::Model, ComposerAction::Plan];
        let found = command_suggestions(TriggerKind::Command, &commands, &actions, "");
        assert_eq!(names(&found), ["model", "plan", "review", "deploy"]);
        assert!(matches!(
            found[0],
            Suggestion::Command {
                action: Some(ComposerAction::Model),
                ..
            }
        ));
    }

    #[test]
    fn skills_are_the_custom_commands() {
        let commands = [
            command("review", false),
            command("deploy", true),
            command("debug", true),
        ];
        let found = command_suggestions(TriggerKind::Skill, &commands, &[], "de");
        assert_eq!(names(&found), ["deploy", "debug"]);
    }

    #[test]
    fn a_lone_slash_command_hoshi_knows_runs_in_place() {
        let actions = [ComposerAction::Model, ComposerAction::Default];
        assert_eq!(
            standalone_action(" /model ", &actions),
            Some(ComposerAction::Model)
        );
        assert_eq!(standalone_action("/plan", &actions), None);
        assert_eq!(standalone_action("/model now", &actions), None);
    }

    #[test]
    fn a_path_with_spaces_is_quoted() {
        assert_eq!(mention_source("src/main.rs"), "@src/main.rs");
        assert_eq!(mention_source("my docs/a.md"), "@\"my docs/a.md\"");
    }
}
