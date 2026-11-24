//! Text fields whose right-click menu says what the app says: the library's own menu only has
//! its own languages.

use gpui_kit::base::input::{
    Copy, Cut, InputBaseState, InputContextMenuCapabilities, InputModeKind, Paste, SelectAll,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState, Textarea, TextareaState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::*;

pub struct EditLabels {
    pub cut: SharedString,
    pub copy: SharedString,
    pub paste: SharedString,
    pub select_all: SharedString,
}

/// Where the text fields' menus take their labels from, from the app: read whenever one opens.
pub struct EditLabelsProvider(pub fn() -> EditLabels);

impl Global for EditLabelsProvider {}

fn labels(cx: &App) -> EditLabels {
    match cx.try_global::<EditLabelsProvider>() {
        Some(provider) => provider.0(),
        None => EditLabels {
            cut: "Cut".into(),
            copy: "Copy".into(),
            paste: "Paste".into(),
            select_all: "Select All".into(),
        },
    }
}

fn items(
    menu: NativeMenu,
    capabilities: InputContextMenuCapabilities,
    labels: EditLabels,
) -> NativeMenu {
    let editable = capabilities.is_editable();
    let copyable = capabilities.is_copyable();
    menu.menu_with_disabled(labels.cut, !(editable && copyable), Box::new(Cut))
        .menu_with_disabled(labels.copy, !copyable, Box::new(Copy))
        .menu_with_disabled(labels.paste, !editable, Box::new(Paste))
        .separator()
        .menu(labels.select_all, Box::new(SelectAll))
}

fn edit_menu<M: InputModeKind>(
    state: &Entity<InputBaseState<M>>,
) -> impl Fn(NativeMenu, &mut Window, &mut App) -> NativeMenu + 'static {
    let state = state.downgrade();
    move |menu, _, cx| match state.upgrade() {
        Some(state) => {
            let capabilities = state.read(cx).context_menu_capabilities();
            items(menu, capabilities, labels(cx))
        }
        None => menu,
    }
}

pub fn text_input(state: &Entity<InputState>) -> Input {
    Input::new(state).context_menu(edit_menu(state))
}

pub fn text_area(state: &Entity<TextareaState>) -> Textarea {
    Textarea::new(state).context_menu(edit_menu(state))
}

pub fn code_editor(state: &Entity<EditorState>) -> Editor {
    Editor::new(state).context_menu(edit_menu(state))
}
