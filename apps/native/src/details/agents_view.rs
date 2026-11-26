use super::*;
use chrono::DateTime;

/// How many agent rows the box shows before its list scrolls; the half row past them hints at
/// the rest.
const ROWS_SHOWN: f32 = 4.5;
const ROW_HEIGHT: f32 = 32.;
const ROW_GAP: f32 = 2.;

/// What the agents box shows of `agents`, newest first as [`ThreadDetails::agents`] lists them.
#[derive(Debug, PartialEq)]
pub(super) struct AgentsSummary {
    /// The rows: the agents that need the user, longest waiting first, then those running,
    /// newest first.
    pub active: Vec<ThreadAgent>,
    pub done: Vec<ThreadAgent>,
}

pub(super) fn summarize(agents: Vec<ThreadAgent>) -> AgentsSummary {
    let (mut waiting, mut running, mut done) = (Vec::new(), Vec::new(), Vec::new());
    for agent in agents {
        match agent.state {
            AgentState::Waiting => waiting.push(agent),
            AgentState::Running => running.push(agent),
            _ => done.push(agent),
        }
    }
    waiting.reverse();
    waiting.extend(running);
    AgentsSummary {
        active: waiting,
        done,
    }
}

impl ThreadDetails {
    /// The box of the agents that worked for the thread, and of the thread that delegated it;
    /// `None` when there are none.
    pub(super) fn agents_panel(
        &self,
        agents: Vec<ThreadAgent>,
        parent: Option<(String, String)>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if agents.is_empty() && parent.is_none() {
            return None;
        }
        let summary = summarize(agents);
        let now = Utc::now();
        let done = (!summary.done.is_empty()).then(|| {
            let count = summary.done.len();
            let tooltip = ended_label(&summary.done);
            self.done_menu(summary.done, now, colors, cx, move |open, colors| {
                div()
                    .id("details-agents-done")
                    .debug_selector(|| "details-agents-done".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .py_0p5()
                    .mr(px(-4.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .when(open, |this| {
                        this.bg(colors.foreground.opacity(0.06))
                            .text_color(colors.foreground)
                    })
                    .hover({
                        let (bg, fg) = (colors.foreground.opacity(0.06), colors.foreground);
                        move |style| style.bg(bg).text_color(fg)
                    })
                    .child(Icon::new(IconName::RotateCcwClock).size_3().flex_none())
                    .child(count.to_string())
                    .child(Icon::new(IconName::ChevronRight).size_3().flex_none())
                    .with_tooltip(Tooltip::new(tooltip.clone()).disabled(open))
            })
        });
        let mut column = panel(self.inline, colors)
            .debug_selector(|| "details-agents".into())
            .relative()
            .child(self.agents_anchor.area())
            .child(panel_header(m::agents(), None, colors).children(done));
        if let Some((id, title)) = parent {
            column = column.child(
                row("details-delegated-by", colors)
                    .child(icon(IconName::GitFork, colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::delegated_by(title)),
                    )
                    .child(icon(IconName::ArrowUpRight, colors))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id.clone(), cx))),
            );
        }
        if !summary.active.is_empty() {
            let scrolls = summary.active.len() as f32 > ROWS_SHOWN;
            let rows = summary
                .active
                .into_iter()
                .map(|agent| Self::agent_row(agent, now, colors, cx));
            column = column.child(
                div()
                    .id("details-agents-list")
                    .debug_selector(|| "details-agents-list".into())
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .max_h(px(ROWS_SHOWN * (ROW_HEIGHT + ROW_GAP)))
                    .overflow_y_scroll()
                    .track_scroll(&self.agents_scroll)
                    .when(scrolls, |this| {
                        let scroll = self.agents_scroll.clone();
                        this.on_scroll(move |event, window, cx| {
                            let delta = event.delta.pixel_delta(window.line_height()).y;
                            let offset = scroll.offset().y;
                            let at_end = if delta < px(0.) {
                                offset <= -scroll.max_offset().y
                            } else {
                                offset >= px(0.)
                            };
                            if !at_end {
                                cx.stop_propagation();
                            }
                        })
                    })
                    .children(rows),
            );
        }
        Some(column)
    }

    /// The menu of the agents that ended, opened by `trigger`.
    fn done_menu<E: IntoElement + 'static>(
        &self,
        agents: Vec<ThreadAgent>,
        now: DateTime<Utc>,
        colors: &Colors,
        cx: &mut Context<Self>,
        trigger: impl Fn(bool, &Colors) -> E + 'static,
    ) -> AnyElement {
        let this = cx.weak_entity();
        let trigger_colors = colors.clone();
        let menu_colors = colors.clone();
        Popover::new("details-agents-done-menu")
            .anchor(self.agents_anchor.clone())
            .side(Side::Left)
            .side_offset(px(rows::BESIDE_BOX))
            .align(Align::Start)
            .trigger(move |open| trigger(open, &trigger_colors))
            .menu(move |menu, _, _| {
                menu.max_list_height(px(360.))
                    .items(agents.iter().cloned().map(|agent| {
                        let this = this.clone();
                        MenuOption::new(
                            SharedString::from(format!("details-agent-option-{}", agent.id)),
                            agent_title(&agent),
                        )
                        .leading(
                            svg()
                                .path(crate::composer::provider_mark(&agent.provider))
                                .size_3p5()
                                .flex_none()
                                .mr_1()
                                .text_color(if agent.state == AgentState::Failed {
                                    menu_colors.status_error
                                } else {
                                    menu_colors.muted_foreground
                                }),
                        )
                        .trailing(
                            div()
                                .text_xs()
                                .text_color(menu_colors.muted_foreground)
                                .child(relative_time(&agent.at, now)),
                        )
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_agent(&agent, cx)).ok();
                        })
                    }))
            })
            .into_any_element()
    }

    fn open_agent(&mut self, agent: &ThreadAgent, cx: &mut Context<Self>) {
        match agent.kind {
            AgentKind::Task => self.open_thread(agent.id.clone(), cx),
            AgentKind::Subagent => self.show_item(agent.id.clone(), cx),
        }
    }

    fn agent_row(
        agent: ThreadAgent,
        now: DateTime<Utc>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selector = format!("details-agent-{}", agent.id);
        let mark_selector = format!("details-agent-mark-{}-{}", agent.id, agent.provider);
        let dot_selector = format!("details-agent-dot-{}", agent.id);
        let title = agent_title(&agent);
        let opened = agent.clone();
        row(
            SharedString::from(format!("details-agent-{}", agent.id)),
            colors,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.open_agent(&opened, cx)))
        .debug_selector(move || selector)
        .child(
            div()
                .relative()
                .flex()
                .flex_none()
                .child(
                    svg()
                        .debug_selector(move || mark_selector)
                        .path(crate::composer::provider_mark(&agent.provider))
                        .size_4()
                        .text_color(crate::composer::provider_tint(&agent.provider, colors)),
                )
                .child(
                    div()
                        .debug_selector(move || dot_selector)
                        .absolute()
                        .right(px(-2.))
                        .bottom(px(-2.))
                        .size(px(6.))
                        .rounded_full()
                        .border_2()
                        .border_color(colors.popover)
                        .bg(agent_dot(agent.state, colors)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(agent.kind == AgentKind::Subagent, |this| {
                    this.text_color(colors.muted_foreground)
                })
                .child(title),
        )
        .child(if agent.state == AgentState::Waiting {
            div()
                .flex_none()
                .rounded_full()
                .px(px(6.))
                .text_size(rems(0.6875))
                .bg(colors.status_waiting.opacity(0.15))
                .text_color(colors.status_waiting)
                .child(m::agent_needs_you())
        } else {
            div()
                .flex_none()
                .text_xs()
                .font_weight(FontWeight::NORMAL)
                .text_color(colors.muted_foreground)
                .child(short_age(&agent.at, now))
        })
    }
}

/// How many agents no longer run: "done" when all finished well, else "ended".
fn ended_label(agents: &[ThreadAgent]) -> String {
    if agents.iter().all(|agent| agent.state == AgentState::Done) {
        m::agents_done(agents.len())
    } else {
        m::agents_ended(agents.len())
    }
}

fn agent_title(agent: &ThreadAgent) -> String {
    if agent.title.is_empty() {
        m::subagent()
    } else {
        agent.title.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentKind, AgentState, ThreadAgent, ended_label, summarize};

    fn agent(id: &str, state: AgentState) -> ThreadAgent {
        ThreadAgent {
            kind: AgentKind::Subagent,
            id: id.into(),
            title: id.into(),
            state,
            provider: "claudeCode".into(),
            at: "2026-11-13T10:00:00Z".into(),
        }
    }

    fn ids(agents: &[ThreadAgent]) -> Vec<&str> {
        agents.iter().map(|agent| agent.id.as_str()).collect()
    }

    #[test]
    fn the_agents_that_need_the_user_come_first_then_the_newest_running() {
        use AgentState::{Done, Failed, Running, Stopped, Waiting};
        let summary = summarize(vec![
            agent("r3", Running),
            agent("w3", Waiting),
            agent("r2", Running),
            agent("d2", Done),
            agent("w2", Waiting),
            agent("s1", Stopped),
            agent("r1", Running),
            agent("w1", Waiting),
            agent("f1", Failed),
        ]);
        assert_eq!(ids(&summary.active), ["w1", "w2", "w3", "r3", "r2", "r1"]);
        assert_eq!(ids(&summary.done), ["d2", "s1", "f1"]);
    }

    #[test]
    fn the_finished_count_says_done_only_when_none_failed_or_stopped() {
        use AgentState::{Done, Failed, Stopped};
        assert_eq!(
            ended_label(&[agent("d1", Done), agent("d2", Done)]),
            "2 done"
        );
        assert_eq!(
            ended_label(&[agent("d1", Done), agent("f1", Failed)]),
            "2 ended"
        );
        assert_eq!(ended_label(&[agent("s1", Stopped)]), "1 ended");
    }

    #[test]
    fn without_active_agents_there_are_no_rows() {
        use AgentState::{Done, Stopped};
        let summary = summarize(vec![agent("d1", Done), agent("s1", Stopped)]);
        assert!(summary.active.is_empty());
        assert_eq!(ids(&summary.done), ["d1", "s1"]);
    }
}
