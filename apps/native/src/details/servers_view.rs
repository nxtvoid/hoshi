use super::*;
use hoshi_ui::prelude::code_language_icon;

impl ThreadDetails {
    /// The box of the servers the thread started and the commands it left running; `None`
    /// when there are none.
    pub(super) fn servers_panel(
        &self,
        servers: Vec<LocalServer>,
        commands_running: usize,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if servers.is_empty() && commands_running == 0 {
            return None;
        }
        let count = servers.len();
        let now = servers::now_millis();
        let rest: Vec<LocalServer> = servers
            .iter()
            .skip(servers::SERVERS_SHOWN)
            .cloned()
            .collect();
        let rows: Vec<_> = servers
            .into_iter()
            .take(servers::SERVERS_SHOWN)
            .map(|server| self.server_row(server, commands_running, now, colors, cx))
            .collect();
        let more = if rest.is_empty() {
            None
        } else {
            Some(self.servers_more(rest, commands_running, now, colors, cx))
        };
        let commands = (commands_running > 0).then(|| {
            row_static("details-commands", colors)
                .font_weight(FontWeight::NORMAL)
                .text_color(colors.muted_foreground)
                .child(icon(IconName::SquareTerminal, colors))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(m::commands_in_background(commands_running)),
                )
        });
        Some(
            panel(self.inline, colors)
                .debug_selector(|| "details-servers".into())
                .relative()
                .child(self.servers_anchor.area())
                .child(panel_header(
                    m::servers(),
                    (count > 0).then(|| m::panel_running(count)),
                    colors,
                ))
                .children(rows)
                .children(more)
                .children(commands)
                .when_some(self.server_error.clone(), |this, error| {
                    this.child(error_line(error, colors))
                }),
        )
    }

    /// One server: what it runs on, its port and process, and its age; pressing it opens it,
    /// and on hover a button stops it.
    fn server_row(
        &self,
        server: LocalServer,
        commands_running: usize,
        now: u64,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("{}:{}", server.pid, server.port);
        let group = SharedString::from(format!("details-server-{key}"));
        let stoppable = servers::stops_from_row(&server, commands_running);
        let stopping = self.stopping == Some((server.pid, server.port));
        let destructive = colors.destructive;
        let web = server.web;
        let address = self
            .shown_thread
            .as_deref()
            .and_then(|thread_id| {
                self.workspace
                    .read(cx)
                    .browsers
                    .read(cx)
                    .last_visit(thread_id, server.port)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| server_url(server.port));
        let lines: Vec<String> = web
            .then(|| m::server_opens(page_path(&address)))
            .into_iter()
            .chain([servers::server_detail(&server, now)])
            .collect();
        let age = server
            .started_at
            .map(|at| servers::server_age(at, now))
            .unwrap_or_default();
        let selector = format!("details-server-{key}");
        let base = if web {
            row(group.clone(), colors)
        } else {
            row_static(group.clone(), colors)
        };
        base.group(group.clone())
            .debug_selector(move || selector)
            .child(runtime_icon(server.process_name.as_deref(), colors))
            .child(div().flex_none().child(m::server_port(server.port)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .children(server.process_name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .when(stoppable && !stopping, |this| {
                        this.group_hover(group.clone(), |style| style.invisible())
                    })
                    .when(stopping, |this| this.invisible())
                    .child(age),
            )
            .when(stoppable, |this| {
                let label = m::server_stop_label(server.port);
                let stop_group = SharedString::from(format!("details-server-stop-{key}"));
                let stop = server.clone();
                this.child(
                    div()
                        .id(SharedString::from(format!("details-server-stop-{key}")))
                        .group(stop_group.clone())
                        .debug_selector({
                            let key = key.clone();
                            move || format!("details-server-stop-{key}")
                        })
                        .absolute()
                        .right(px(4.))
                        .top(px(4.))
                        .flex()
                        .size_6()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .text_color(colors.muted_foreground)
                        .when(!stopping, |this| {
                            this.invisible()
                                .group_hover(group.clone(), |style| style.visible())
                        })
                        .when(self.stopping.is_none(), |this| {
                            this.cursor_pointer().hover(move |style| {
                                style.bg(destructive.opacity(0.1)).text_color(destructive)
                            })
                        })
                        .child(if stopping {
                            Spinner::new()
                                .icon(IconName::LoaderCircle)
                                .color(destructive)
                                .with_size(px(14.))
                                .into_any_element()
                        } else {
                            div()
                                .size(px(10.))
                                .border(px(1.5))
                                .rounded(px(3.))
                                .border_color(colors.muted_foreground)
                                .group_hover(stop_group.clone(), |style| {
                                    style.border_color(destructive)
                                })
                                .into_any_element()
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.stop_server(&stop, cx)
                        }))
                        .with_tooltip(Tooltip::new(label)),
                )
            })
            .relative()
            .when(web, |this| {
                this.on_click(move |_, _, cx| crate::markdown::open_web_link(&address, false, cx))
            })
            .with_tooltip(
                Tooltip::element(move |_, _| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .children(lines.iter().cloned().map(|line| div().child(line)))
                })
                .side(Side::Left)
                .side_offset(px(rows::BESIDE_ROW)),
            )
            .into_any_element()
    }

    /// "+N more", opening the servers past the first ones as the same rows.
    fn servers_more(
        &self,
        rest: Vec<LocalServer>,
        commands_running: usize,
        now: u64,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = rest.len();
        let trigger_colors = colors.clone();
        let content_colors = colors.clone();
        let this = cx.weak_entity();
        Popover::new("details-servers-more")
            .anchor(self.servers_anchor.clone())
            .side(Side::Left)
            .side_offset(px(rows::BESIDE_BOX))
            .align(Align::Start)
            .w(px(CARD_WIDTH))
            .p_1()
            .gap(px(2.))
            .trigger(move |open| {
                row("details-servers-more", &trigger_colors)
                    .debug_selector(|| "details-servers-more".into())
                    .when(open, |this| {
                        this.bg(trigger_colors.foreground.opacity(0.05))
                    })
                    .font_weight(FontWeight::NORMAL)
                    .text_color(trigger_colors.muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::servers_more(count)),
                    )
                    .child(Icon::new(IconName::ChevronRight).size_3p5().flex_none())
            })
            .content(move |_, _, cx| {
                this.update(cx, |this, cx| {
                    div()
                        .debug_selector(|| "details-servers-more-list".into())
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(rest.into_iter().map(|server| {
                            this.server_row(server, commands_running, now, &content_colors, cx)
                        }))
                        .into_any_element()
                })
                .unwrap_or_else(|_| div().into_any_element())
            })
            .into_any_element()
    }
}

fn server_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

/// What follows the host in `url`, `/` when nothing does.
fn page_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    match rest.find('/') {
        Some(at) => rest[at..].to_owned(),
        None => "/".to_owned(),
    }
}

/// The icon of what a server runs on, in its language's color; a globe when that is unknown.
fn runtime_icon(process: Option<&str>, colors: &Colors) -> AnyElement {
    match servers::runtime_language(process) {
        Some(language) => {
            let (name, tint) = code_language_icon(language);
            svg()
                .path(hoshi_ui::assets::file_icon(name))
                .size_4()
                .flex_none()
                .text_color(tint(colors))
                .into_any_element()
        }
        None => icon(IconName::Globe, colors).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::page_path;

    #[test]
    fn a_row_tells_the_page_it_opens() {
        assert_eq!(
            page_path("http://localhost:18400/main/bodega"),
            "/main/bodega"
        );
        assert_eq!(page_path("http://localhost:5173"), "/");
        assert_eq!(page_path("http://localhost:5173/?tab=2"), "/?tab=2");
    }
}
