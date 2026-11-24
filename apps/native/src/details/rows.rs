use super::prefs::{EditorMark, editor_mark};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Editor, ForgeKind};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;

pub const CARD_WIDTH: f32 = 280.;
pub const CARD_GAP: f32 = 12.;
pub const CARD_CLEARANCE: f32 = 32.;
/// Room around the floating boxes for their shadows, inside the card's gap.
pub const SHADOW_ROOM: f32 = 8.;
/// Below this chat width the card would crowd the chat, so it becomes a popover.
pub const INLINE_MIN_WIDTH: f32 = CARD_WIDTH + CARD_GAP + CARD_CLEARANCE * 2. + 560.;

const ROW_TEXT: f32 = 13.;
/// How far a tooltip on the left of a row stands from it, so it clears the box.
pub const BESIDE_ROW: f32 = 16.;
/// How far a menu on the left of a box stands from its edge, as a row's tooltip does.
pub const BESIDE_BOX: f32 = 11.;

/// A row of the card: one line, an icon, its text, and a hover tint.
pub fn row(id: impl Into<ElementId>, colors: &Colors) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    row_static(id, colors)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
}

/// A row that does nothing when pressed.
pub fn row_static(id: impl Into<ElementId>, colors: &Colors) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .h_8()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(10.))
        .rounded_lg()
        .px(px(10.))
        .text_size(px(ROW_TEXT))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.foreground.opacity(0.8))
}

/// A small square button at the end of a row.
pub fn icon_button(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    open: bool,
    colors: &Colors,
) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    icon_slot(id, content)
        .cursor_pointer()
        .when(open, |this| this.bg(hover))
        .hover(move |style| style.bg(hover))
}

/// Where an icon button sits, without its press: for one that can't be pressed now.
pub fn icon_slot(id: impl Into<ElementId>, content: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .size_6()
        .items_center()
        .justify_center()
        .rounded_md()
        .child(content)
}

/// The editor's own mark, as the editors show it.
pub fn editor_icon(editor: Editor, colors: &Colors) -> AnyElement {
    match editor_mark(editor) {
        EditorMark::Colored(name) => img(brand::editor(name))
            .size_4()
            .flex_none()
            .into_any_element(),
        EditorMark::Tinted(name) => svg()
            .path(brand::editor(name))
            .size_4()
            .flex_none()
            .text_color(colors.foreground)
            .into_any_element(),
        EditorMark::Generic => icon(IconName::Code, colors).into_any_element(),
    }
}

/// The forge's own mark, as the project palette shows it.
pub fn forge_mark(forge: ForgeKind, colors: &Colors) -> AnyElement {
    match forge {
        ForgeKind::Github => Icon::new(IconName::Github)
            .size_4()
            .flex_none()
            .text_color(colors.foreground)
            .into_any_element(),
        ForgeKind::Bitbucket => svg()
            .path(brand::BITBUCKET)
            .size_4()
            .flex_none()
            .text_color(colors.link)
            .into_any_element(),
    }
}

pub fn icon(name: IconName, colors: &Colors) -> Icon {
    Icon::new(name)
        .size_4()
        .flex_none()
        .text_color(colors.muted_foreground)
}

/// One of the card's boxes. Floating over the chat it carries its own surface and shadow;
/// inside the popover only its border.
pub fn panel(floating: bool, colors: &Colors) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(2.))
        .p_1()
        .rounded_xl()
        .border_1()
        .border_color(colors.border.opacity(0.6))
        .when(floating, |this| {
            this.bg(colors.popover.opacity(0.85)).shadow_lg()
        })
}

/// A box's title, with what it holds on the right.
pub fn panel_header(
    title: impl Into<SharedString>,
    detail: Option<String>,
    colors: &Colors,
) -> Div {
    div()
        .flex()
        .flex_none()
        .h_7()
        .items_center()
        .gap_2()
        .px_2()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(div().flex_1().min_w_0().truncate().child(title.into()))
        .children(detail.map(|detail| div().flex_none().child(detail)))
}

/// `+12 −3`, as the card shows a diff's size.
pub fn diff_count(additions: u32, deletions: u32, colors: &Colors) -> Div {
    div()
        .flex()
        .flex_none()
        .gap_1()
        .text_xs()
        .font_weight(FontWeight::NORMAL)
        .child(
            div()
                .text_color(colors.pr_open)
                .child(format!("+{additions}")),
        )
        .child(
            div()
                .text_color(colors.status_error)
                .child(format!("−{deletions}")),
        )
}

pub fn error_line(text: impl Into<SharedString>, colors: &Colors) -> Div {
    div()
        .px(px(10.))
        .pt_1()
        .pb_1()
        .text_xs()
        .text_color(colors.destructive)
        .child(text.into())
}
