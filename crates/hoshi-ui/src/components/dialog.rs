//! Modal dialogs, shaped like Base UI's `Dialog` and `AlertDialog`.
//!
//! Every dialog of a window renders in one window-level layer, [`DialogLayer`], above the app:
//! call [`DialogLayer::init`] once after `gpui_kit::init`, before opening windows. A dialog is
//! either controlled, rendered like any element with [`Dialog::open`] and
//! [`Dialog::on_open_change`], or opened from anywhere with [`DialogLayer::open`] and
//! [`DialogLayer::confirm`].

mod button;
mod confirm;
mod fade;
mod layer;

pub use button::{DialogButton, DialogButtonSize, DialogButtonVariant};
pub use confirm::{ConfirmChoice, ConfirmRequest, ConfirmTone};
pub use layer::{DIALOG_PRIORITY, DialogHandle, DialogLayer};

use gpui_kit::*;
use std::rc::Rc;

gpui_kit::actions!(hoshi_dialog, [Dismiss, Close]);

const KEY_CONTEXT: &str = "HoshiDialog";

/// Why a dialog asks to change its open state, as Base UI's `eventDetails.reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenChangeReason {
    /// A press on the backdrop. Alert dialogs never report it.
    OutsidePress,
    EscapeKey,
    /// A [`DialogButton::close`] or the corner close button.
    ClosePress,
    /// [`DialogHandle::close`].
    Imperative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenChange {
    pub open: bool,
    pub reason: OpenChangeReason,
}

/// The popup's maximum width; it never gets closer than 16px to the window's edges.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DialogSize {
    Xs,
    #[default]
    Sm,
    Md,
    Lg,
    Xl,
    Custom(Pixels),
}

impl DialogSize {
    pub fn max_width(self) -> Pixels {
        match self {
            Self::Xs => px(320.),
            Self::Sm => px(384.),
            Self::Md => px(448.),
            Self::Lg => px(512.),
            Self::Xl => px(576.),
            Self::Custom(width) => width,
        }
    }
}

/// What takes focus when the dialog opens.
#[derive(Clone, Default)]
pub enum InitialFocus {
    /// The first tab stop inside the popup, or the popup when it has none.
    #[default]
    FirstTabbable,
    Popup,
    Handle(FocusHandle),
}

/// What takes focus back when the dialog closes.
#[derive(Clone, Default)]
pub enum FinalFocus {
    /// Whatever held focus when the dialog opened.
    #[default]
    Previous,
    Handle(FocusHandle),
    None,
}

pub(crate) type OpenChangeHandler = Rc<dyn Fn(&OpenChange, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Surface {
    #[default]
    Standard,
    Confirm,
    Page,
    Palette,
    Wizard,
}

/// A modal dialog: title, description, a body that scrolls when the window is too short, and a
/// footer of actions. Rendered anywhere, it shows in the window's [`DialogLayer`].
///
/// ```ignore
/// Dialog::new("rename")
///     .open(self.renaming)
///     .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
///         this.renaming = change.open;
///         cx.notify();
///     }))
///     .title("Rename thread")
///     .child(self.input.clone())
///     .footer(DialogButton::new("cancel").label("Cancel").variant(DialogButtonVariant::Outline).close())
///     .footer(DialogButton::new("save").label("Save").on_click(cx.listener(Self::save)))
/// ```
#[derive(IntoElement)]
pub struct Dialog {
    pub(crate) id: ElementId,
    pub(crate) open: bool,
    pub(crate) alert: bool,
    pub(crate) size: DialogSize,
    pub(crate) title: Option<SharedString>,
    pub(crate) description: Option<SharedString>,
    pub(crate) body: Vec<AnyElement>,
    pub(crate) footer: Vec<AnyElement>,
    pub(crate) close_button: Option<bool>,
    pub(crate) close_label: Option<SharedString>,
    pub(crate) initial_focus: InitialFocus,
    pub(crate) final_focus: FinalFocus,
    pub(crate) on_open_change: Option<OpenChangeHandler>,
    pub(crate) surface: Surface,
}

impl Dialog {
    /// The id keys a controlled dialog across frames; keep it stable.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            open: false,
            alert: false,
            size: DialogSize::default(),
            title: None,
            description: None,
            body: Vec::new(),
            footer: Vec::new(),
            close_button: None,
            close_label: None,
            initial_focus: InitialFocus::default(),
            final_focus: FinalFocus::default(),
            on_open_change: None,
            surface: Surface::default(),
        }
    }

    /// Makes it an alert dialog: a backdrop press doesn't close it, and it has no corner close
    /// button unless asked for.
    pub fn alert(mut self) -> Self {
        self.alert = true;
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Makes it a large modal for a whole page, such as Settings: it fills the window up to
    /// 1280 × 960, and its children lay out the whole popup, header and close button included.
    pub fn page(mut self) -> Self {
        self.surface = Surface::Page;
        self
    }

    /// Makes it a command palette: near the top of the window instead of centered, and its
    /// children lay out the whole popup, with no padding of their own.
    pub fn palette(mut self) -> Self {
        self.surface = Surface::Palette;
        self
    }

    /// Makes it a wizard: 768px wide, a window drag strip above it, and a backdrop press that
    /// does nothing. Its children lay out the whole popup, as with [`Dialog::page`].
    pub fn wizard(mut self) -> Self {
        self.surface = Surface::Wizard;
        self.size = DialogSize::Custom(px(768.));
        self
    }

    /// Called when the dialog asks to open or close. A controlled dialog stays as it is until
    /// the caller renders it with the new `open`; one opened with [`DialogLayer::open`] closes.
    pub fn on_open_change(
        mut self,
        handler: impl Fn(&OpenChange, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }

    pub fn size(mut self, size: DialogSize) -> Self {
        self.size = size;
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds an action to the footer, right-aligned in the order added.
    pub fn footer(mut self, action: impl IntoElement) -> Self {
        self.footer.push(action.into_any_element());
        self
    }

    /// Shows or hides the corner close button; dialogs show it, alert dialogs don't.
    pub fn close_button(mut self, shown: bool) -> Self {
        self.close_button = Some(shown);
        self
    }

    /// The corner close button's accessible name.
    pub fn close_label(mut self, label: impl Into<SharedString>) -> Self {
        self.close_label = Some(label.into());
        self
    }

    pub fn initial_focus(mut self, focus: InitialFocus) -> Self {
        self.initial_focus = focus;
        self
    }

    pub fn final_focus(mut self, focus: FinalFocus) -> Self {
        self.final_focus = focus;
        self
    }

    pub(crate) fn surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }

    pub(crate) fn shows_close_button(&self) -> bool {
        self.surface == Surface::Standard && self.close_button.unwrap_or(!self.alert)
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        DialogLayer::stage(self, window, cx);
        Empty
    }
}

#[cfg(test)]
mod tests {
    use super::{Dialog, DialogSize};
    use gpui_kit::px;

    #[test]
    fn sizes_follow_the_web_max_widths() {
        assert_eq!(DialogSize::default().max_width(), px(384.));
        assert_eq!(DialogSize::Xs.max_width(), px(320.));
        assert_eq!(DialogSize::Lg.max_width(), px(512.));
        assert_eq!(DialogSize::Custom(px(400.)).max_width(), px(400.));
    }

    #[test]
    fn only_dialogs_show_a_close_button_by_default() {
        assert!(Dialog::new("d").shows_close_button());
        assert!(!Dialog::new("a").alert().shows_close_button());
        assert!(
            Dialog::new("a")
                .alert()
                .close_button(true)
                .shows_close_button()
        );
        assert!(!Dialog::new("d").close_button(false).shows_close_button());
    }
}
