use super::*;
use chrono::DateTime;

/// How many provider marks the summary row shows before a `+N`.
const MARKS_SHOWN: usize = 5;
/// How many agents that need the user get a row of their own before the rest go in a menu.
const WAITING_SHOWN: usize = 3;
const MARK_SIZE: f32 = 18.;

/// What the agents box shows of `agents`, newest first as [`ThreadDetails::agents`] lists them.
#[derive(Debug, PartialEq)]
pub(super) struct AgentsSummary {
    /// The active agents for the marks: the ones that need the user first.
    pub marks: Vec<ThreadAgent>,
    pub active: usize,
    pub previous: usize,
    /// The rows under the marks: the agents that need the user, longest waiting first, else the
    /// newest one running.
    pub rows: Vec<ThreadAgent>,
    /// The agents that need the user past [`WAITING_SHOWN`].
    pub more_waiting: Vec<ThreadAgent>,
    pub waiting: Vec<ThreadAgent>,
    pub running: Vec<ThreadAgent>,
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
    let longest_waiting: Vec<ThreadAgent> = waiting.iter().rev().cloned().collect();
    let rows = if longest_waiting.is_empty() {
        running.first().cloned().into_iter().collect()
    } else {
        longest_waiting
            .iter()
            .take(WAITING_SHOWN)
            .cloned()
            .collect()
    };
    AgentsSummary {
        marks: waiting
            .iter()
            .chain(&running)
            .take(MARKS_SHOWN)
            .cloned()
            .collect(),
        active: waiting.len() + running.len(),
        previous: done.len(),
        rows,
        more_waiting: longest_waiting.into_iter().skip(WAITING_SHOWN).collect(),
        waiting,
        running,
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
        let any = summary.active + summary.previous > 0;
        let all = any.then(|| {
            let groups = vec![
                (m::agents_need_you(), summary.waiting.clone()),
                (m::agents_running_heading(), summary.running.clone()),
                (m::agents_previous(), summary.done.clone()),
            ];
            self.agents_menu(
                "details-agents-all",
                groups,
                now,
                colors,
                cx,
                |open, colors| {
                    div()
                        .id("details-agents-show-all")
                        .debug_selector(|| "details-agents-show-all".into())
                        .flex()
                        .items_center()
                        .gap_0p5()
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
                        .child(m::agents_show_all())
                        .child(Icon::new(IconName::ChevronRight).size_3().flex_none())
                },
            )
        });
        let mut column = panel(self.inline, colors)
            .debug_selector(|| "details-agents".into())
            .relative()
            .child(self.agents_anchor.area())
            .child(panel_header(m::agents(), None, colors).pr_2().children(all));
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
        if any {
            column = column.child(self.summary_row(&summary, colors, cx));
        }
        for agent in summary.rows.iter().cloned() {
            column = column.child(Self::agent_row(agent, now, colors, cx));
        }
        if !summary.more_waiting.is_empty() {
            let count = summary.more_waiting.len();
            let more = self.agents_menu(
                "details-agents-waiting",
                vec![(m::agents_need_you(), summary.more_waiting.clone())],
                now,
                colors,
                cx,
                move |open, colors| {
                    row("details-agents-more-waiting", colors)
                        .when(open, |this| this.bg(colors.foreground.opacity(0.05)))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(m::agents_more_need_you(count)),
                        )
                        .child(Icon::new(IconName::ChevronRight).size_3p5().flex_none())
                },
            );
            column = column.child(more);
        }
        Some(column)
    }

    /// The marks of the active agents, waiting ones ringed, and how many run and are done.
    fn summary_row(
        &self,
        summary: &AgentsSummary,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let text = match (summary.active, summary.previous) {
            (0, done) => m::agents_done(done),
            (active, 0) => m::panel_running(active),
            (active, done) => m::agents_summary(m::agents_done(done), m::panel_running(active)),
        };
        let extra = summary.active.saturating_sub(summary.marks.len());
        let marks = summary
            .marks
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, agent)| Self::mark(index, agent, colors, cx).into_any_element())
            .chain((extra > 0).then(|| {
                div()
                    .flex_none()
                    .h(px(MARK_SIZE))
                    .ml(px(-4.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.popover)
                    .text_size(rems(0.625))
                    .text_color(colors.muted_foreground)
                    .child(m::agents_more(extra))
                    .into_any_element()
            }));
        row_static("details-agents-summary", colors)
            .debug_selector(|| "details-agents-summary".into())
            .child(div().flex().flex_none().items_center().children(marks))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .child(text),
            )
    }

    /// An active agent's provider mark, ringed when it needs the user; pressing it opens it.
    fn mark(
        index: usize,
        agent: ThreadAgent,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let waiting = agent.state == AgentState::Waiting;
        let selector = format!("details-agent-mark-{}", agent.id);
        let title = agent_title(&agent);
        div()
            .id(SharedString::from(format!(
                "details-agent-mark-{}",
                agent.id
            )))
            .debug_selector(move || selector)
            .flex()
            .flex_none()
            .size(px(MARK_SIZE))
            .when(index > 0, |this| this.ml(px(-4.)))
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(if waiting {
                colors.status_waiting
            } else {
                colors.border
            })
            .bg(colors.popover)
            .cursor_pointer()
            .child(
                svg()
                    .path(crate::composer::provider_mark(&agent.provider))
                    .size(px(11.))
                    .text_color(crate::composer::provider_tint(&agent.provider, colors)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.open_agent(&agent, cx)))
            .with_tooltip(Tooltip::new(title))
    }

    /// A menu of `groups` of agents, each under its heading, opened by `trigger`.
    fn agents_menu<E: IntoElement + 'static>(
        &self,
        id: &'static str,
        groups: Vec<(String, Vec<ThreadAgent>)>,
        now: DateTime<Utc>,
        colors: &Colors,
        cx: &mut Context<Self>,
        trigger: impl Fn(bool, &Colors) -> E + 'static,
    ) -> AnyElement {
        let this = cx.weak_entity();
        let trigger_colors = colors.clone();
        let menu_colors = colors.clone();
        Popover::new(id)
            .anchor(self.agents_anchor.clone())
            .side(Side::Left)
            .side_offset(px(rows::BESIDE_BOX))
            .align(Align::Start)
            .trigger(move |open| trigger(open, &trigger_colors))
            .menu(move |mut menu, _, _| {
                menu = menu.max_list_height(px(360.));
                for (heading, agents) in groups.iter().filter(|(_, agents)| !agents.is_empty()) {
                    menu = menu.heading(m::agents_group(agents.len(), heading));
                    menu = menu.items(agents.iter().cloned().map(|agent| {
                        let this = this.clone();
                        let tint = crate::composer::provider_tint(&agent.provider, &menu_colors);
                        let side = if agent.state == AgentState::Waiting {
                            div()
                                .rounded_full()
                                .px(px(6.))
                                .text_size(rems(0.6875))
                                .bg(menu_colors.status_waiting.opacity(0.15))
                                .text_color(menu_colors.status_waiting)
                                .child(m::agent_needs_you())
                        } else {
                            div()
                                .text_xs()
                                .text_color(menu_colors.muted_foreground)
                                .child(relative_time(&agent.at, now))
                        };
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
                                .text_color(if agent.state.is_active() {
                                    tint
                                } else {
                                    menu_colors.muted_foreground
                                }),
                        )
                        .trailing(side)
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_agent(&agent, cx)).ok();
                        })
                    }));
                }
                menu
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
                .child(relative_time(&agent.at, now))
        })
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
    use super::{AgentKind, AgentState, ThreadAgent, summarize};

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
    fn the_agents_that_need_the_user_come_first_longest_waiting_first() {
        use AgentState::{Done, Running, Waiting};
        let summary = summarize(vec![
            agent("r3", Running),
            agent("w5", Waiting),
            agent("r2", Running),
            agent("w4", Waiting),
            agent("w3", Waiting),
            agent("w2", Waiting),
            agent("r1", Running),
            agent("w1", Waiting),
            agent("d1", Done),
        ]);
        assert_eq!(ids(&summary.marks), ["w5", "w4", "w3", "w2", "w1"]);
        assert_eq!(ids(&summary.rows), ["w1", "w2", "w3"]);
        assert_eq!(ids(&summary.more_waiting), ["w4", "w5"]);
        assert_eq!((summary.active, summary.previous), (8, 1));
    }

    #[test]
    fn without_one_waiting_the_newest_running_agent_shows() {
        use AgentState::{Done, Running, Stopped};
        let summary = summarize(vec![
            agent("r2", Running),
            agent("r1", Running),
            agent("d1", Done),
        ]);
        assert_eq!(ids(&summary.rows), ["r2"]);
        assert!(summary.more_waiting.is_empty());

        let summary = summarize(vec![agent("d1", Done), agent("s1", Stopped)]);
        assert!(summary.rows.is_empty() && summary.marks.is_empty());
        assert_eq!((summary.active, summary.previous), (0, 2));
    }
}
