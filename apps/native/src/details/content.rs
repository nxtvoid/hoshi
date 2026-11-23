use super::*;

impl ThreadDetails {
    /// The header's part: how many servers run while the details are out of sight, and the
    /// button that shows them.
    pub fn header_toggle(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(subject) = self.subject(cx) else {
            return div().into_any_element();
        };
        let colors = cx.hoshi().clone();
        let count = match &subject {
            Subject::Thread(thread_id) => self
                .workspace
                .read(cx)
                .servers
                .get(thread_id)
                .map_or(0, Vec::len),
            Subject::Draft(_) | Subject::ChatDraft(_) => 0,
        };
        let notice = (!self.shown(cx) && count > 0).then(|| {
            servers_notice(count, &colors).on_click(cx.listener(|this, _, _, cx| {
                if this.inline {
                    this.set_open(true, cx);
                } else {
                    this.popover_open = true;
                }
                cx.notify();
            }))
        });
        let toggle = |open: bool| {
            Button::new("toggle-details")
                .icon(IconName::List)
                .size(ButtonSize::IconSm)
                .variant(if open {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .tooltip(Tooltip::new(m::thread_details()).side(Side::Bottom))
        };
        let button = if self.inline {
            toggle(self.is_open(cx))
                .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
                .into_any_element()
        } else {
            let this = cx.weak_entity();
            Popover::new("details-popover")
                .align(Align::End)
                .w(px(CARD_WIDTH))
                .p(px(6.))
                .gap_0()
                .open(self.popover_open)
                .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                    this.popover_open = change.open;
                    cx.notify();
                }))
                .trigger(toggle)
                .content(move |_, window, cx| {
                    this.update(cx, |this, cx| this.subject_content(&subject, window, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .children(notice)
            .child(button)
            .into_any_element()
    }

    pub(super) fn subject_content(
        &mut self,
        subject: &Subject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match subject {
            Subject::Thread(thread_id) => self.content(thread_id, window, cx),
            Subject::Draft(project_id) => self.draft_content(project_id, cx),
            Subject::ChatDraft(thread_id) => self.chat_draft_content(thread_id, cx),
        }
    }

    fn chat_draft_content(&mut self, thread_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let provider = self.workspace.read(cx).choices.model.provider.clone();
        let mut column = div().flex().flex_col().gap(px(2.));
        if let Some(folder) = self.chat_folders.get(thread_id) {
            column = column.child(self.folder_row(folder, None, &colors));
        }
        if let Some(editor) = self.open_target() {
            column = column.child(self.editor_row(editor, &colors, cx));
        }
        if let Some(limits) = self.usage_limits(&provider, cx) {
            column = column
                .child(separator(&colors))
                .child(usage::section(&limits, &colors));
        }
        column
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            })
            .into_any_element()
    }

    pub(super) fn draft_content(&mut self, project_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let Some(root) = workspace
            .model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
        else {
            return div().into_any_element();
        };
        let branch = workspace
            .project_branches
            .get(project_id)
            .cloned()
            .flatten();
        let provider = workspace.choices.model.provider.clone();
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.folder_row(&root, None, &colors));
        if let Some(editor) = self.open_target() {
            column = column.child(self.editor_row(editor, &colors, cx));
        }
        if let Some(branch) = branch {
            let mono = cx.theme().mono_font_family.clone();
            column = column.child(separator(&colors)).child(
                row_static("details-branch", &colors)
                    .debug_selector(|| "details-draft-branch".into())
                    .child(icon(IconName::GitBranch, &colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono)
                            .text_xs()
                            .child(branch),
                    ),
            );
        }
        if let Some(limits) = self.usage_limits(&provider, cx) {
            column = column
                .child(separator(&colors))
                .child(usage::section(&limits, &colors));
        }
        column
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            })
            .into_any_element()
    }

    pub(super) fn content(
        &mut self,
        thread_id: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(thread) = self.workspace.read(cx).model.thread(thread_id).cloned() else {
            return div().into_any_element();
        };
        self.ensure_fresh(&thread, cx);
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let root = thread
            .project_id
            .as_deref()
            .and_then(|project| workspace.model.project(project))
            .map(|project| project.workspace_root.clone());
        let pull_request = workspace.pull_requests.get(thread_id).cloned();
        let servers = workspace
            .servers
            .get(thread_id)
            .cloned()
            .unwrap_or_default();
        let commands_running = workspace
            .background_tasks
            .get(thread_id)
            .copied()
            .unwrap_or(0);
        let phase = workspace.git_phases.get(thread_id).copied();
        let parent = thread
            .parent_thread_id
            .as_deref()
            .and_then(|parent| workspace.model.thread(parent))
            .map(|parent| (parent.id.clone(), parent.title.clone()));
        let path = thread
            .worktree_path
            .clone()
            .or(root)
            .or_else(|| self.chat_folders.get(thread_id).cloned())
            .unwrap_or_default();
        let agents = self.agents(thread_id, cx);
        let changes = self.changes.get(thread_id).cloned();
        let running = self.running.get(thread_id).copied();
        if running.is_some() {
            self.tick_in(Duration::from_secs(1), cx);
        }
        if !servers.is_empty() {
            self.tick_in(servers::age_tick(&servers, servers::now_millis()), cx);
        }

        let mut column = div().flex().flex_col().gap(px(2.)).child(self.folder_row(
            &path,
            worktree_badge(&thread),
            &colors,
        ));
        if let Some(editor) = self.open_target() {
            column = column.child(self.editor_row(editor, &colors, cx));
        }
        if thread.branch.is_some() || pull_request.is_some() || changes.is_some() {
            column = column.child(separator(&colors));
        }
        if let Some(branch) = thread.branch.clone() {
            column = column.child(self.branch_row(branch, &thread, &colors, cx));
        }
        if let Some(pull_request) = &pull_request {
            column = column.child(
                pull_request_row(pull_request, thread.watching_pull_request, &colors)
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::PullRequest, cx))),
            );
        }
        if thread.project_id.is_some()
            && let Some(status) = self.statuses.get(thread_id).cloned()
        {
            let open = pull_request.as_ref().is_some_and(|pull_request| {
                matches!(
                    pull_request.state,
                    PullRequestState::Open | PullRequestState::Draft
                )
            });
            let working = thread.active_turn_id.is_some();
            column = column.child(self.git_row(
                thread_id, &status, open, working, running, phase, &colors, cx,
            ));
        }
        if let Some(changes) = changes {
            column = column.child(
                row("details-diff", &colors)
                    .child(icon(IconName::FileDiff, &colors))
                    .child(div().flex_none().child(m::surface_diff()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors.muted_foreground)
                            .child(m::diff_scope_uncommitted()),
                    )
                    .child(diff_count(changes.additions, changes.deletions, &colors))
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::Diff, cx))),
            );
        }
        column = self.agents_section(column, agents, parent, &colors, cx);
        column = self.servers_section(column, servers, commands_running, &colors, cx);
        if let Some(limits) = self.usage_limits(usage::provider_of(&thread), cx) {
            column = column
                .child(separator(&colors))
                .child(usage::section(&limits, &colors));
        }
        column
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            })
            .into_any_element()
    }

    /// The limits of `provider` when the card shows them, as last read.
    pub(super) fn usage_limits(&self, provider: &str, cx: &App) -> Option<UsageLimits> {
        let workspace = self.workspace.read(cx);
        if !workspace.settings.usage_limits_in_details {
            return None;
        }
        let known = crate::providers::known(provider)?;
        if !known.enabled(&workspace.settings.providers) {
            return None;
        }
        let limits = workspace.usage_limits(provider);
        let state = section_state(
            workspace.providers.accounts.get(provider).copied(),
            limits,
            false,
            false,
        );
        limits
            .filter(|limits| state == SectionState::Limits && !limits.windows.is_empty())
            .cloned()
    }
}
