use crate::ClickHandler;
use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// The small muted title of a group in a menu.
#[derive(IntoElement)]
pub struct MenuHeading {
    text: SharedString,
}

impl MenuHeading {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for MenuHeading {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .px_1p5()
            .py_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(cx.hoshi().muted_foreground)
            .child(self.text)
    }
}

/// A row of options of which one is on, on a muted track.
#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    segments: Vec<Segment>,
}

pub struct Segment {
    content: AnyElement,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl Segment {
    pub fn new(content: impl IntoElement) -> Self {
        Self {
            content: content.into_any_element(),
            selected: false,
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Segmented {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            segments: Vec::new(),
        }
    }

    pub fn segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        div()
            .id(self.id)
            .flex()
            .gap_0p5()
            .p_0p5()
            .rounded_lg()
            .bg(colors.muted)
            .children(
                self.segments
                    .into_iter()
                    .enumerate()
                    .map(|(index, segment)| {
                        let selected = segment.selected;
                        let foreground = colors.foreground;
                        div()
                            .id(index)
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(if selected {
                                colors.foreground
                            } else {
                                colors.muted_foreground
                            })
                            .when(selected, |item| item.bg(colors.popover).shadow_xs())
                            .when(!selected, |item| {
                                item.hover(move |style| style.text_color(foreground))
                            })
                            .when_some(segment.on_click, |item, handler| {
                                item.on_click(move |event, window, cx| handler(event, window, cx))
                            })
                            .child(segment.content)
                    }),
            )
    }
}
