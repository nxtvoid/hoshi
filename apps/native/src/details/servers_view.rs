use super::*;

impl ThreadDetails {
    pub(super) fn servers_section(
        &self,
        column: Div,
        servers: Vec<LocalServer>,
        commands_running: usize,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        if servers.is_empty() {
            return column;
        }
        let now = servers::now_millis();
        let hidden = servers.len().saturating_sub(servers::SERVERS_SHOWN);
        let shown = if self.servers_expanded {
            servers.len()
        } else {
            servers::SERVERS_SHOWN
        };
        let mono = colors.foreground.opacity(0.8);
        let rows: Vec<_> =
            servers
                .into_iter()
                .take(shown)
                .map(|server| {
                    let key = format!("{}:{}", server.pid, server.port);
                    let stoppable = servers::stops_from_row(&server, commands_running);
                    let stopping = self.stopping == Some((server.pid, server.port));
                    let label = m::server_stop_label(server.port);
                    let destructive = colors.destructive;
                    div()
                        .id(SharedString::from(format!("details-server-{key}")))
                        .relative()
                        .flex()
                        .min_h_10()
                        .min_w_0()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py_1()
                        .when(stoppable, |this| this.pr_10())
                        .child(
                            div()
                                .flex_none()
                                .size(px(7.))
                                .rounded_full()
                                .bg(colors.status_done),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(mono)
                                        .child(m::server_address(server.port)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(servers::server_detail(&server, now)),
                                ),
                        )
                        .when(stoppable, |this| {
                            this.child(
                                div()
                                    .id(SharedString::from(format!("details-server-stop-{key}")))
                                    .absolute()
                                    .right(px(6.))
                                    .top(px(8.))
                                    .flex()
                                    .size_6()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_color(destructive)
                                    .when(self.stopping.is_none(), |this| {
                                        this.cursor_pointer()
                                            .hover(move |style| style.bg(destructive.opacity(0.1)))
                                    })
                                    .child(if stopping {
                                        Spinner::new()
                                            .icon(IconName::LoaderCircle)
                                            .color(destructive)
                                            .with_size(px(14.))
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::Square).size_3p5().into_any_element()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_server(&server, cx)
                                    }))
                                    .with_tooltip(Tooltip::new(label)),
                            )
                        })
                })
                .collect();
        let hover = colors.foreground.opacity(0.05);
        column
            .child(separator(colors))
            .child(section_label(m::servers(), colors))
            .children(rows)
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .id("details-servers-more")
                        .flex()
                        .h_8()
                        .items_center()
                        .rounded_lg()
                        .px(px(10.))
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(if self.servers_expanded {
                            m::servers_show_less()
                        } else {
                            m::servers_show_more(hidden)
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.servers_expanded = !this.servers_expanded;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.server_error.clone(), |this, error| {
                this.child(error_line(error, colors))
            })
    }
}
