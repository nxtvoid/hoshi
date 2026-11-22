use super::*;

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
        let agent_row = |agent: ThreadAgent, cx: &mut Context<Self>| {
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
            base.debug_selector(move || selector)
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .child(icon(
                            match agent.kind {
                                AgentKind::Task => IconName::GitFork,
                                AgentKind::Subagent => IconName::Bot,
                            },
                            colors,
                        ))
                        .child(
                            div()
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
        };
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
            let rows: Vec<_> = active
                .into_iter()
                .map(|agent| agent_row(agent, cx))
                .collect();
            column = column.child(
                div()
                    .id("details-agents-active")
                    .flex()
                    .flex_col()
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .children(rows),
            );
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
                let rows: Vec<_> = previous
                    .into_iter()
                    .map(|agent| agent_row(agent, cx))
                    .collect();
                column = column.child(
                    div()
                        .id("details-agents-previous-list")
                        .flex()
                        .flex_col()
                        .max_h(px(192.))
                        .overflow_y_scroll()
                        .children(rows),
                );
            }
        }
        column
    }
}
