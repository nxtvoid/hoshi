use super::*;
use crate::format::compact_number;
use rows::{icon_button, icon_slot};

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

    /// The card's boxes, one under another, without the ones that would be empty.
    fn boxes(boxes: impl IntoIterator<Item = Option<Div>>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(boxes.into_iter().flatten())
            .into_any_element()
    }

    fn usage_panel(
        &self,
        limits: Option<UsageLimits>,
        context: Option<usage::ContextMeter>,
        colors: &Colors,
    ) -> Option<Div> {
        let shown = limits.as_ref().is_some_and(shows_limits);
        (shown || context.is_some()).then(|| {
            panel(self.inline, colors)
                .debug_selector(|| "details-usage".into())
                .child(usage::section(limits.as_ref(), context, colors))
        })
    }

    /// The button that compacts the thread's context, pressable once that is worth a turn and
    /// the agent is at rest.
    fn compact_button(
        &self,
        worth: bool,
        working: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph = icon(IconName::Minimize2, colors);
        if working || !worth {
            return icon_slot("details-compact", glyph)
                .debug_selector(|| "details-compact".into())
                .opacity(0.4)
                .into_any_element();
        }
        icon_button("details-compact", glyph, false, colors)
            .debug_selector(|| "details-compact".into())
            .on_click(cx.listener(|this, _, _, cx| this.compact_context(cx)))
            .with_tooltip(Tooltip::new(m::compact_context()))
            .into_any_element()
    }

    fn chat_draft_content(&mut self, thread_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let provider = self.workspace.read(cx).choices.model.provider.clone();
        let place = self.chat_folders.get(thread_id).cloned().map(|folder| {
            panel(self.inline, &colors)
                .child(self.folder_row(&folder, None, &colors, cx))
                .when_some(self.error.clone(), |this, error| {
                    this.child(error_line(error, &colors))
                })
        });
        let usage = self.usage_panel(self.usage_limits(&provider, cx), None, &colors);
        Self::boxes([place, usage])
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
        let place = panel(self.inline, &colors)
            .child(self.folder_row(&root, None, &colors, cx))
            .when_some(branch, |this, branch| {
                this.child(
                    row_static("details-branch", &colors)
                        .debug_selector(|| "details-draft-branch".into())
                        .child(icon(IconName::GitBranch, &colors))
                        .child(div().flex_1().min_w_0().truncate().child(branch)),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            });
        let usage = self.usage_panel(self.usage_limits(&provider, cx), None, &colors);
        Self::boxes([Some(place), usage])
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
        let context_usage = workspace
            .context_usages
            .get(thread_id)
            .copied()
            .filter(|_| workspace.settings.context_meter == hoshi_core::ContextMeterPlace::Details);
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
        if self.pending_git.contains_key(thread_id) {
            self.tick_in(Duration::from_millis(33), cx);
        }
        if !servers.is_empty() {
            self.tick_in(servers::age_tick(&servers, servers::now_millis()), cx);
        }

        let pull_request_open = pull_request.as_ref().is_some_and(|pull_request| {
            matches!(
                pull_request.state,
                PullRequestState::Open | PullRequestState::Draft
            )
        });
        let working = thread.active_turn_id.is_some();
        let status = thread
            .project_id
            .is_some()
            .then(|| self.statuses.get(thread_id).cloned())
            .flatten();
        let git_row = status.as_ref().and_then(|status| {
            self.git_row(
                thread_id,
                status,
                pull_request_open,
                working,
                running,
                phase,
                &colors,
                cx,
            )
        });
        let mut place = panel(self.inline, &colors)
            .debug_selector(|| "details-place".into())
            .child(self.folder_row(&path, worktree_badge(&thread), &colors, cx));
        let detached = status
            .as_ref()
            .is_some_and(|status| status.branch.is_none());
        let branch = thread
            .branch
            .clone()
            .or_else(|| detached.then(m::git_detached_head));
        if let Some(branch) = branch {
            let clean = status
                .as_ref()
                .is_some_and(|status| !status.has_changes && git_row.is_none());
            let menu = status
                .as_ref()
                .filter(|_| running.is_none())
                .map(|status| self.git_menu(status, pull_request_open, working, &colors, cx));
            place = place.child(self.branch_row(branch, &thread, clean, menu, &colors));
        }
        if let Some(pull_request) = &pull_request {
            place = place.child(
                pull_request_row(pull_request, thread.watching_pull_request, &colors)
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::PullRequest, cx))),
            );
        }
        if let Some(changes) = changes.filter(|changes| changes.files > 0) {
            place = place.child(
                row("details-diff", &colors)
                    .child(icon(IconName::FileDiff, &colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::turn_changed_files(changes.files as usize)),
                    )
                    .child(diff_count(changes.additions, changes.deletions, &colors))
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::Diff, cx))),
            );
        }
        let place = place
            .children(git_row)
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            });
        let limits = self.usage_limits(usage::provider_of(&thread), cx);
        let context = context_usage.map(|usage| {
            let percent = crate::composer::context_percent(&usage);
            let worth = hoshi_core::worth_compacting(&usage);
            let used = compact_number(usage.used_tokens);
            let mut lines = vec![match percent {
                Some(percent) => m::context_window_usage(
                    compact_number(usage.max_tokens.unwrap_or_default()),
                    percent.round() as u32,
                    &used,
                ),
                None => m::context_tokens_used(&used),
            }];
            if working {
                lines.push(m::error_thread_busy());
            } else if !worth {
                lines.push(m::compact_context_from(compact_number(
                    hoshi_core::compaction_threshold(&usage),
                )));
            }
            usage::ContextMeter {
                percent,
                value: match percent {
                    Some(percent) => m::usage_percent(percent.round() as u8),
                    None => used,
                },
                lines,
                compact: self.compact_button(worth, working, &colors, cx),
            }
        });
        let usage = self.usage_panel(limits, context, &colors);
        let agents = self.agents_panel(agents, parent, &colors, cx);
        let servers = self.servers_panel(servers, commands_running, &colors, cx);
        Self::boxes([Some(place), agents, servers, usage])
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
            .filter(|limits| state == SectionState::Limits && shows_limits(limits))
            .cloned()
    }
}

fn shows_limits(limits: &UsageLimits) -> bool {
    !limits.windows.is_empty() || limits.extra_usage.is_some()
}
