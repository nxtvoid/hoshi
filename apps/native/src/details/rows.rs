use super::prefs::{EditorMark, editor_mark};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::Editor;
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;

pub const CARD_WIDTH: f32 = 280.;
pub const CARD_GAP: f32 = 12.;
pub const CARD_CLEARANCE: f32 = 32.;
/// Below this chat width the card would crowd the chat, so it becomes a popover.
pub const INLINE_MIN_WIDTH: f32 = CARD_WIDTH + CARD_GAP + CARD_CLEARANCE * 2. + 560.;

const ROW_TEXT: f32 = 13.;

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

/// The part of a split row that runs its action.
pub fn split_primary(id: impl Into<ElementId>, colors: &Colors) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    div()
        .id(id)
        .flex()
        .flex_1()
        .h_8()
        .min_w_0()
        .items_center()
        .gap(px(10.))
        .rounded_l_lg()
        .px(px(10.))
        .text_size(px(ROW_TEXT))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.foreground.opacity(0.8))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
}

/// A row whose main part runs an action and whose arrow opens alternatives.
pub fn split_row(
    id: impl Into<ElementId>,
    primary: impl IntoElement,
    menu: Option<AnyElement>,
    colors: &Colors,
) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.02);
    let divider = colors.border.opacity(0.7);
    div()
        .id(id)
        .group("split")
        .flex()
        .flex_none()
        .w_full()
        .items_center()
        .rounded_lg()
        .hover(move |style| style.bg(hover))
        .child(primary)
        .when_some(menu, |this, menu| {
            this.child(
                div()
                    .h_4()
                    .w_px()
                    .flex_none()
                    .group_hover("split", move |style| style.bg(divider)),
            )
            .child(menu)
        })
}

/// The arrow of a split row, which opens its menu.
pub fn split_arrow(id: impl Into<ElementId>, open: bool, colors: &Colors) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    div()
        .id(id)
        .flex()
        .flex_none()
        .size_8()
        .items_center()
        .justify_center()
        .rounded_r_lg()
        .cursor_pointer()
        .when(open, |this| this.bg(hover))
        .hover(move |style| style.bg(hover))
        .child(icon(IconName::ChevronDown, colors))
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

pub fn icon(name: IconName, colors: &Colors) -> Icon {
    Icon::new(name)
        .size_4()
        .flex_none()
        .text_color(colors.muted_foreground)
}

pub fn separator(colors: &Colors) -> Div {
    div()
        .flex_none()
        .mx(px(10.))
        .my_1()
        .h_px()
        .bg(colors.border.opacity(0.7))
}

pub fn section_label(text: impl Into<SharedString>, colors: &Colors) -> Div {
    div()
        .flex_none()
        .px(px(10.))
        .pt_1()
        .pb(px(2.))
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(text.into())
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
