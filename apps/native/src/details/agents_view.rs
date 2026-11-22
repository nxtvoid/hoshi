use super::*;
use chrono::DateTime;

const AGENT_ROW_HEIGHT: Rems = rems(2.);
const ACTIVE_AGENTS_SHOWN: f32 = 7.5;
const PREVIOUS_AGENTS_SHOWN: f32 = 6.;

impl ThreadDetails {
    pub(super) fn agents_section(
        &self,
        column: Div,
        agents: Vec<ThreadAgent>,
        parent: Option<(String, String)>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        if agents.is_empty() && parent.is_none() {
            return column;
        }
        let (active, previous): (Vec<_>, Vec<_>) = agents
            .into_iter()
            .partition(|agent| agent.state.is_active());
        let now = Utc::now();
        let mut column = column
            .child(separator(colors))
            .child(section_label(m::agents(), colors));
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
                    .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id.clone(), cx))),
            );
        }
        if !active.is_empty() {
            column = column.child(Self::agent_list(
                "details-agents-active",
                active,
                ACTIVE_AGENTS_SHOWN,
                now,
                colors,
                cx,
            ));
        }
        if !previous.is_empty() {
            let open = self.previous_agents_open;
            column = column.child(
                row("details-agents-previous", colors)
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::agents_previous()),
                    )
                    .child(div().text_xs().child(previous.len().to_string()))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .flex_none(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.previous_agents_open = !this.previous_agents_open;
                        cx.notify();
                    })),
            );
            if open {
                column = column.child(Self::agent_list(
                    "details-agents-previous-list",
                    previous,
                    PREVIOUS_AGENTS_SHOWN,
                    now,
                    colors,
                    cx,
                ));
            }
        }
        column
    }

    /// A list of agents that builds only the rows in view, however many there are.
    fn agent_list(
        id: &'static str,
        agents: Vec<ThreadAgent>,
        shown: f32,
        now: DateTime<Utc>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = agents.len();
        let colors = colors.clone();
        uniform_list(
            id,
            count,
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| Self::agent_row(agents[index].clone(), now, &colors, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .w_full()
        .h(AGENT_ROW_HEIGHT * (count as f32).min(shown))
    }

    fn agent_row(
        agent: ThreadAgent,
        now: DateTime<Utc>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let base = match agent.kind {
            AgentKind::Task => {
                let id = agent.id.clone();
                row(
                    SharedString::from(format!("details-agent-{}", agent.id)),
                    colors,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id.clone(), cx)))
            }
            AgentKind::Subagent => {
                let id = agent.id.clone();
                row(
                    SharedString::from(format!("details-agent-{}", agent.id)),
                    colors,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.show_item(id.clone(), cx)))
            }
        };
        let selector = format!("details-agent-{}", agent.id);
        let mark_selector = format!("details-agent-mark-{}-{}", agent.id, agent.provider);
        let dot_selector = format!("details-agent-dot-{}", agent.id);
        base.debug_selector(move || selector)
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
                    .child(if agent.title.is_empty() {
                        m::subagent()
                    } else {
                        agent.title.clone()
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .child(relative_time(&agent.at, now)),
            )
    }
}
