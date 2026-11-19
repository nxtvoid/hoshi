//! A message the user sent, as the web shows it: its images above it, the files its text
//! doesn't mention beside them, its text in a bubble with the mentions as chips, and on hover
//! when it was sent and a copy button.

use crate::m;
use crate::markdown::mentions::{self, Mention, MentionKind};
use crate::markdown::{CopyButton, message_markdown, show_image};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::MessageAttachment;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::{ActiveTheme as _, MessageBubble, PressSelectsNothing as _};

const IMAGE_TILE: Pixels = px(96.);
/// How tall a long message shows until it is opened.
const COLLAPSED_HEIGHT: Pixels = px(320.);
/// A message only this much taller than the collapsed height shows whole.
const COLLAPSE_SLACK: Pixels = px(48.);

/// A long message cut to its first lines, faded, with a button that shows it whole.
#[derive(IntoElement)]
struct Collapsible {
    id: SharedString,
    content: AnyElement,
}

impl RenderOnce for Collapsible {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let overflows = window.use_keyed_state(
            SharedString::from(format!("{}-overflows", self.id)),
            cx,
            |_, _| false,
        );
        let expanded = window.use_keyed_state(
            SharedString::from(format!("{}-expanded", self.id)),
            cx,
            |_, _| false,
        );
        let long = *overflows.read(cx);
        let open = *expanded.read(cx);
        let collapsed = long && !open;
        let measured = overflows.clone();
        let content = div().relative().child(self.content).child(
            canvas(
                move |bounds, _, cx| {
                    let long = bounds.size.height > COLLAPSED_HEIGHT + COLLAPSE_SLACK;
                    if *measured.read(cx) != long {
                        let measured = measured.clone();
                        cx.defer(move |cx| {
                            measured.update(cx, |overflows, cx| {
                                *overflows = long;
                                cx.notify();
                            });
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let fade = colors.secondary;
        let selector = self.id.to_string();
        div()
            .debug_selector(move || selector)
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .when(collapsed, |clip| {
                        clip.max_h(COLLAPSED_HEIGHT).overflow_hidden()
                    })
                    .child(content)
                    .when(collapsed, |clip| {
                        clip.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(rems(4.))
                                .bg(linear_gradient(
                                    180.,
                                    linear_color_stop(fade.opacity(0.), 0.),
                                    linear_color_stop(fade, 1.),
                                )),
                        )
                    }),
            )
            .when(long, |bubble| {
                bubble.child(
                    div()
                        .id(self.id.clone())
                        .mt_1()
                        .mr(px(-4.))
                        .self_end()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(colors.background.opacity(0.7))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground.opacity(0.8))
                        .hover(|style| style.bg(colors.background).text_color(colors.foreground))
                        .child(if open {
                            m::message_show_less()
                        } else {
                            m::message_show_full()
                        })
                        .press_selects_nothing()
                        .on_click(move |_, _, cx| {
                            expanded.update(cx, |expanded, cx| {
                                *expanded = !*expanded;
                                cx.notify();
                            });
                        }),
                )
            })
    }
}

/// The files whose `[File #n]` the text doesn't show, which show above it instead.
fn files_outside_text<'a>(
    text: &str,
    attachments: &'a [MessageAttachment],
) -> Vec<&'a MessageAttachment> {
    attachments
        .iter()
        .filter(|attachment| matches!(attachment, MessageAttachment::File { .. }))
        .enumerate()
        .filter(|(index, _)| !text.contains(&format!("[File #{}]", index + 1)))
        .map(|(_, file)| file)
        .collect()
}

/// `note` shows under the bubble, as for a message that never reached the agent.
#[allow(clippy::too_many_arguments)]
pub fn user_message(
    id: &str,
    text: &str,
    attachments: &[MessageAttachment],
    steer: bool,
    created_at: &str,
    action: Option<AnyElement>,
    note: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    if text.is_empty() && attachments.is_empty() {
        return div().into_any_element();
    }
    let colors = cx.hoshi().clone();
    let group = SharedString::from(format!("message-{id}"));
    let images: Vec<AnyElement> = attachments
        .iter()
        .filter_map(|attachment| match attachment {
            MessageAttachment::Image {
                id: image_id,
                name,
                mime_type,
                ..
            } => {
                let picture = crate::markdown::message_image(image_id, mime_type, cx);
                let (image_id, name, mime_type) =
                    (image_id.clone(), name.clone(), mime_type.clone());
                Some(
                    div()
                        .id(SharedString::from(format!("image-{image_id}")))
                        .size(IMAGE_TILE)
                        .overflow_hidden()
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.muted)
                        .when(picture.is_some(), |tile| tile.cursor_pointer())
                        .children(picture.map(|picture| {
                            img(ImageSource::Image(picture))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                        }))
                        .press_selects_nothing()
                        .on_click(move |_, window, cx| {
                            show_image(
                                image_id.clone(),
                                mime_type.clone(),
                                name.clone(),
                                window,
                                cx,
                            )
                        })
                        .into_any_element(),
                )
            }
            _ => None,
        })
        .collect();
    let files: Vec<AnyElement> = files_outside_text(text, attachments)
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            let mention = Mention {
                kind: MentionKind::AttachedFile,
                text: SharedString::default(),
                attachment: Some(file.clone()),
            };
            mentions::chip(&mention, index, px(14.), cx).into_any_element()
        })
        .collect();
    let time = crate::subagents::clock_time(created_at);
    let muted = colors.muted_foreground;
    div()
        .group(group.clone())
        .flex()
        .flex_col()
        .items_end()
        .gap_1p5()
        .when(steer, |message| {
            message.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(Icon::new(IconName::CornerDownRight).size_3p5())
                    .child(m::message_steer()),
            )
        })
        .when(!images.is_empty(), |message| {
            message.child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_1p5()
                    .max_w(relative(0.85))
                    .children(images),
            )
        })
        .when(!files.is_empty(), |message| {
            message.child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_1p5()
                    .max_w(relative(0.85))
                    .text_sm()
                    .children(files),
            )
        })
        .when(!text.is_empty(), |message| {
            message.child(
                div().w_full().child(
                    MessageBubble::new().child(Collapsible {
                        id: SharedString::from(format!("collapse-{id}")),
                        content: message_markdown(id.to_owned(), text, attachments, &colors)
                            .into_any_element(),
                    }),
                ),
            )
        })
        .children(note)
        .child(
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .h_6()
                .text_xs()
                .text_color(muted)
                .opacity(0.)
                .group_hover(group, |actions| actions.opacity(1.))
                .children(time.map(|time| div().px_1().child(time)))
                .children(action)
                .when(!text.is_empty(), |actions| {
                    actions.child(CopyButton::new(
                        SharedString::from(format!("copy-{id}")),
                        text.to_owned().into(),
                        m::copy_message().into(),
                    ))
                }),
        )
        .into_any_element()
}

/// Under a message the turn ended before reading: that it wasn't sent, and a button that puts
/// it back in the composer.
pub fn not_sent_note(id: &str, send_again: impl Fn(&mut App) + 'static, cx: &App) -> AnyElement {
    let colors = cx.hoshi().clone();
    let selector = format!("not-sent-{id}");
    let button = format!("send-again-{id}");
    let foreground = colors.foreground;
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(m::message_not_sent())
        .child("·")
        .child(
            div()
                .id(SharedString::from(button.clone()))
                .debug_selector(move || button.clone())
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .press_selects_nothing()
                .on_click(move |_, _, cx| send_again(cx))
                .child(m::message_send_again()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::files_outside_text;
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AnyView, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Point,
        Render, StyleRefinement, Styled as _, TestAppContext, VisualTestContext, Window,
        WindowBounds, WindowOptions, div, px, size,
    };
    use hoshi_core::MessageAttachment;

    fn file(id: &str) -> MessageAttachment {
        MessageAttachment::File {
            id: id.into(),
            name: format!("{id}.pdf"),
            mime_type: "application/pdf".into(),
            size: 1,
        }
    }

    #[test]
    fn files_the_text_mentions_stay_in_it() {
        let attachments = [file("a"), file("b")];
        let outside = files_outside_text("see [File #2]", &attachments);
        assert_eq!(outside, [&attachments[0]]);
    }

    struct Message {
        lines: usize,
    }

    impl Render for Message {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let text = "A long question.\n\n".repeat(self.lines);
            div().size_full().child(super::user_message(
                "u1",
                &text,
                &[],
                false,
                "2026-11-12T10:00:00Z",
                None,
                None,
                cx,
            ))
        }
    }

    struct Cached(Entity<Message>);

    impl Render for Cached {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(AnyView::from(self.0.clone()).cached(StyleRefinement::default().size_full()))
        }
    }

    #[gpui_kit::test]
    fn a_long_message_collapses_inside_a_cached_view(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let (window, message) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(2000.)),
                })),
                ..Default::default()
            };
            let mut shown = None;
            let (window, _) = gpui_kit::open_window(options, cx, |_, cx| {
                let message = cx.new(|_| Message { lines: 1 });
                shown = Some(message.clone());
                cx.new(|_| Cached(message))
            })
            .unwrap();
            (window, shown.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        let redraw = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            for _ in 0..3 {
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.run_until_parked();
            }
        };
        redraw(&mut cx);
        message.update(&mut cx, |message, cx| {
            message.lines = 40;
            cx.notify();
        });
        redraw(&mut cx);
        let bubble = cx.debug_bounds("collapse-u1").expect("the bubble shows");
        assert!(bubble.size.height < px(500.), "{bubble:?}");
    }
}
