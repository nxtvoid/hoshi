use super::confirm::{self, ConfirmChoice, ConfirmRequest};
use super::fade::{self, Fade};
use super::{
    Close, Dialog, DialogButton, DialogButtonSize, DialogButtonVariant, Dismiss, FinalFocus,
    InitialFocus, KEY_CONTEXT, OpenChange, OpenChangeHandler, OpenChangeReason, Surface,
};
use crate::assets::IconName;
use crate::components::DragRegion as _;
use crate::components::popover::{any_popover_open, popover_at};
use crate::theme::ActiveTheme;
use futures::channel::oneshot;
use gpui_kit::base::{FocusTrapElement as _, Root, RootPlugin, TextSelection};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Duration;

/// The dialogs' paint priority among deferred elements: above the app's own deferred content,
/// below popups (`gpui_kit::base::POPUP_PRIORITY`), so a popover opened inside a dialog shows
/// above it.
pub const DIALOG_PRIORITY: usize = 50;

const MISSING: &str = "the window has no dialog layer: call hoshi_ui::DialogLayer::init after \
                       gpui_kit::init, before opening windows";

type Builder = Rc<dyn Fn(Dialog, &mut Window, &mut App) -> Dialog>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Controlled(ElementId),
    Opened(u64),
}

impl Key {
    fn element_id(&self) -> ElementId {
        match self {
            Self::Controlled(id) => id.clone(),
            Self::Opened(id) => ElementId::NamedInteger("hoshi-dialog".into(), *id),
        }
    }
}

struct Entry {
    key: Key,
    builder: Option<Builder>,
    popup: FocusHandle,
    return_to: Option<WeakFocusHandle>,
    inner_focus: Option<WeakFocusHandle>,
    final_focus: FinalFocus,
    on_open_change: Option<OpenChangeHandler>,
    parent: Option<Key>,
    fade: Fade,
    closing: bool,
    generation: u64,
    needs_focus: bool,
    seen: u64,
    on_closed: Option<Box<dyn FnOnce()>>,
    confirm: bool,
}

/// The window's dialogs, stacked in the order they opened, the newest on top. Only the newest
/// open one takes input: the app and the dialogs below it get no pointer, scroll or keyboard
/// input until it closes.
pub struct DialogLayer {
    entries: Vec<Entry>,
    staged: HashMap<Key, Dialog>,
    frame: u64,
    next_id: u64,
    pending_confirms: VecDeque<(ConfirmRequest, oneshot::Sender<Option<ConfirmChoice>>)>,
}

/// A dialog opened with [`DialogLayer::open`].
#[derive(Clone)]
pub struct DialogHandle {
    layer: WeakEntity<DialogLayer>,
    id: u64,
}

impl DialogHandle {
    /// Closes it as Base UI's `actionsRef.close()`, reported as
    /// [`Imperative`](OpenChangeReason::Imperative).
    pub fn close(&self, window: &mut Window, cx: &mut App) {
        if let Some(layer) = self.layer.upgrade() {
            DialogLayer::request_close(
                &layer,
                &Key::Opened(self.id),
                OpenChangeReason::Imperative,
                window,
                cx,
            );
        }
    }

    pub fn is_open(&self, cx: &App) -> bool {
        self.layer.upgrade().is_some_and(|layer| {
            layer
                .read(cx)
                .entry(&Key::Opened(self.id))
                .is_some_and(|entry| !entry.closing)
        })
    }
}

impl DialogLayer {
    /// Mounts the layer in every window opened after this, and binds Escape.
    pub fn init(cx: &mut App) {
        Root::register_plugin::<Self>(cx, Self::new);
        cx.bind_keys([KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT))]);
    }

    fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            staged: HashMap::new(),
            frame: 0,
            next_id: 0,
            pending_confirms: VecDeque::new(),
        }
    }

    fn entity(window: &Window, cx: &App) -> Option<Entity<Self>> {
        window.root::<Root>().flatten()?.read(cx).plugin::<Self>()
    }

    /// Opens a dialog from anywhere. `build` runs on every frame while it shows; the [`Dialog`]
    /// it gets is already keyed, and its `open` is ignored.
    pub fn open(
        window: &mut Window,
        cx: &mut App,
        build: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
    ) -> DialogHandle {
        let layer = Self::entity(window, cx).expect(MISSING);
        let id = layer.update(cx, |layer, cx| {
            let id = layer.next_id();
            layer.push(Key::Opened(id), Some(Rc::new(build)), window, cx);
            cx.notify();
            id
        });
        DialogHandle {
            layer: layer.downgrade(),
            id,
        }
    }

    /// Asks in an alert dialog; resolves to whether the user went ahead. Escape, the cancel
    /// button, or the window closing resolve to `false`. A confirm asked while another one shows
    /// waits for it to close.
    pub fn confirm(request: ConfirmRequest, window: &mut Window, cx: &mut App) -> Task<bool> {
        let choice = Self::choose(request, window, cx);
        cx.background_spawn(async move { choice.await == Some(ConfirmChoice::Confirm) })
    }

    /// Asks as [`confirm`](Self::confirm) does, with the request's alternative as a second way
    /// ahead; resolves to the way taken, `None` when cancelled.
    pub fn choose(
        request: ConfirmRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Option<ConfirmChoice>> {
        let (sender, receiver) = oneshot::channel();
        let layer = Self::entity(window, cx).expect(MISSING);
        layer.update(cx, |layer, cx| {
            if layer.has_open_confirm() {
                layer.pending_confirms.push_back((request, sender));
            } else {
                layer.open_confirm(request, sender, window, cx);
                cx.notify();
            }
        });
        cx.background_spawn(async move { receiver.await.unwrap_or(None) })
    }

    /// How many dialogs show and take input, closing ones aside.
    pub fn open_count(window: &Window, cx: &App) -> usize {
        Self::entity(window, cx).map_or(0, |layer| {
            layer
                .read(cx)
                .entries
                .iter()
                .filter(|entry| !entry.closing)
                .count()
        })
    }

    pub(crate) fn stage(dialog: Dialog, window: &mut Window, cx: &mut App) {
        let layer = Self::entity(window, cx).expect(MISSING);
        layer.update(cx, |layer, cx| layer.sync(dialog, window, cx));
    }

    fn sync(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        let key = Key::Controlled(dialog.id.clone());
        let changed = match self.position(&key) {
            None if dialog.open => {
                self.push(key.clone(), None, window, cx);
                true
            }
            Some(ix) if dialog.open && self.entries[ix].closing && !self.parent_closing(ix) => {
                self.reopen(ix, window, cx);
                true
            }
            Some(ix) if !dialog.open && !self.entries[ix].closing => {
                self.close(&key, window, cx);
                true
            }
            _ => false,
        };
        if changed {
            window.defer(cx, |window, _| window.refresh());
        }
        let frame = self.frame;
        if let Some(entry) = self.entry_mut(&key) {
            entry.seen = frame;
            entry.on_open_change = dialog.on_open_change.clone();
            self.staged.insert(key, dialog);
        }
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn duration(cx: &App) -> Duration {
        if cx.reduce_motion() {
            Duration::ZERO
        } else {
            fade::DURATION
        }
    }

    fn position(&self, key: &Key) -> Option<usize> {
        self.entries.iter().position(|entry| entry.key == *key)
    }

    fn entry(&self, key: &Key) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key == *key)
    }

    fn entry_mut(&mut self, key: &Key) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.key == *key)
    }

    fn has_open_confirm(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.confirm && !entry.closing)
    }

    fn parent_closing(&self, ix: usize) -> bool {
        self.entries[ix]
            .parent
            .as_ref()
            .is_some_and(|parent| self.entry(parent).is_none_or(|parent| parent.closing))
    }

    fn top_open_popup(&self) -> Option<FocusHandle> {
        self.entries
            .iter()
            .rev()
            .find(|entry| !entry.closing)
            .map(|entry| entry.popup.clone())
    }

    fn push(
        &mut self,
        key: Key,
        builder: Option<Builder>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = self
            .entries
            .iter()
            .rev()
            .find(|entry| !entry.closing && entry.popup.contains_focused(window, cx))
            .map(|entry| entry.key.clone());
        self.entries.push(Entry {
            key,
            builder,
            popup: cx.focus_handle(),
            return_to: window.focused(cx).map(|focused| focused.downgrade()),
            inner_focus: None,
            final_focus: FinalFocus::default(),
            on_open_change: None,
            parent,
            fade: Fade::opening(cx.background_executor().now()),
            closing: false,
            generation: 0,
            needs_focus: true,
            seen: self.frame,
            on_closed: None,
            confirm: false,
        });
        TextSelection::clear(window, cx);
    }

    fn reopen(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let duration = Self::duration(cx);
        let return_to = window.focused(cx).map(|focused| focused.downgrade());
        let entry = &mut self.entries[ix];
        entry.closing = false;
        entry.generation += 1;
        entry.needs_focus = true;
        entry.return_to = return_to;
        entry.fade.toward(1., now, duration);
    }

    fn open_confirm(
        &mut self,
        request: ConfirmRequest,
        sender: oneshot::Sender<Option<ConfirmChoice>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = Rc::new(Cell::new(None));
        let confirm_focus = cx.focus_handle();
        let id = self.next_id();
        let builder: Builder = {
            let answer = answer.clone();
            Rc::new(move |dialog, _, cx| {
                confirm::build(dialog, &request, &answer, &confirm_focus, cx)
            })
        };
        self.push(Key::Opened(id), Some(builder), window, cx);
        if let Some(entry) = self.entries.last_mut() {
            entry.confirm = true;
            entry.on_closed = Some(Box::new(move || {
                let _ = sender.send(answer.get());
            }));
        }
    }

    /// Asks the dialog to close: a controlled one only hears about it, the rest close.
    fn request_close(
        layer: &Entity<Self>,
        key: &Key,
        reason: OpenChangeReason,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(handler) = layer
            .read(cx)
            .entry(key)
            .filter(|entry| !entry.closing)
            .map(|entry| entry.on_open_change.clone())
        else {
            return;
        };
        if matches!(key, Key::Opened(_)) {
            layer.update(cx, |layer, cx| {
                layer.close(key, window, cx);
                cx.notify();
            });
        }
        if let Some(handler) = handler {
            handler(
                &OpenChange {
                    open: false,
                    reason,
                },
                window,
                cx,
            );
        }
    }

    /// Starts the exit: the dialog and the ones opened from it stop taking input, focus goes
    /// back, and they unmount once faded out.
    fn close(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        if self.entry(key).is_none_or(|entry| entry.closing) {
            return;
        }
        let children: Vec<Key> = self
            .entries
            .iter()
            .filter(|entry| entry.parent.as_ref() == Some(key) && !entry.closing)
            .map(|entry| entry.key.clone())
            .collect();
        for child in children.iter().rev() {
            self.close(child, window, cx);
        }
        let Some(ix) = self.position(key) else {
            return;
        };
        let now = cx.background_executor().now();
        let duration = Self::duration(cx);
        let entry = &mut self.entries[ix];
        entry.closing = true;
        entry.generation += 1;
        entry.needs_focus = false;
        entry.fade.toward(0., now, duration);
        let remaining = entry.fade.remaining(now, duration);
        let generation = entry.generation;
        let focused = window.focused(cx);
        let had_focus = entry.popup.contains_focused(window, cx)
            || (focused.is_some()
                && entry.inner_focus.as_ref().and_then(|weak| weak.upgrade()) == focused);
        let target = match &entry.final_focus {
            FinalFocus::Previous => entry.return_to.as_ref().and_then(|weak| weak.upgrade()),
            FinalFocus::Handle(handle) => Some(handle.clone()),
            FinalFocus::None => None,
        };
        let confirm = entry.confirm;
        if let Some(done) = entry.on_closed.take() {
            done();
        }
        if had_focus {
            match target.or_else(|| self.top_open_popup()) {
                Some(target) => window.focus(&target, cx),
                None => window.blur(cx),
            }
        }
        if remaining.is_zero() {
            self.entries.retain(|entry| entry.key != *key);
        } else {
            let key = key.clone();
            cx.spawn(async move |layer, cx| {
                cx.background_executor().timer(remaining).await;
                layer
                    .update(cx, |layer, cx| layer.remove_closed(&key, generation, cx))
                    .ok();
            })
            .detach();
        }
        if confirm
            && !self.has_open_confirm()
            && let Some((request, sender)) = self.pending_confirms.pop_front()
        {
            self.open_confirm(request, sender, window, cx);
        }
    }

    /// Unmounts the controlled dialogs nobody rendered this frame, as React would.
    fn unmount_unseen(&mut self, frame: u64, window: &mut Window, cx: &mut Context<Self>) {
        let unmounted: Vec<Key> = self
            .entries
            .iter()
            .filter(|entry| matches!(entry.key, Key::Controlled(_)) && entry.seen != frame)
            .map(|entry| entry.key.clone())
            .collect();
        for key in &unmounted {
            self.close(key, window, cx);
            self.entries.retain(|entry| entry.key != *key);
        }
        if !unmounted.is_empty() {
            cx.notify();
        }
    }

    fn remove_closed(&mut self, key: &Key, generation: u64, cx: &mut Context<Self>) {
        let before = self.entries.len();
        self.entries.retain(|entry| {
            !(entry.key == *key && entry.closing && entry.generation == generation)
        });
        if self.entries.len() != before {
            cx.notify();
        }
    }
}

impl RootPlugin for DialogLayer {
    fn prepare(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.staged.clear();
        self.frame += 1;
    }
}

impl Render for DialogLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entries.is_empty() {
            return Empty.into_any_element();
        }
        let layer = cx.entity();
        if self
            .entries
            .iter()
            .any(|entry| matches!(entry.key, Key::Controlled(_)))
        {
            let layer = layer.downgrade();
            let frame = self.frame;
            window.defer(cx, move |window, cx| {
                layer
                    .update(cx, |layer, cx| layer.unmount_unseen(frame, window, cx))
                    .ok();
            });
        }
        let topmost = self.entries.iter().rposition(|entry| !entry.closing);
        if let Some(entry) = topmost.map(|ix| &self.entries[ix])
            && !entry.needs_focus
        {
            let layer = layer.downgrade();
            let key = entry.key.clone();
            window.defer(cx, move |window, cx| {
                let popup = layer.upgrade().and_then(|layer| {
                    let layer = layer.read(cx);
                    let top = layer.entries.iter().rev().find(|entry| !entry.closing)?;
                    (top.key == key && !top.needs_focus).then(|| top.popup.clone())
                });
                if let Some(popup) = popup
                    && !popup.contains_focused(window, cx)
                {
                    focus_initial(&popup, &InitialFocus::FirstTabbable, window, cx);
                }
            });
        }
        let dimmed = self
            .entries
            .iter()
            .position(|entry| !entry.closing)
            .unwrap_or(0);
        let viewport = window.viewport_size();
        let slots: Vec<Slot> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| Slot {
                layer: layer.clone(),
                key: entry.key.clone(),
                builder: entry.builder.clone(),
                index,
                topmost: topmost == Some(index),
                dimmed: index == dimmed,
            })
            .collect();
        deferred(
            anchored()
                .position(point(px(0.), px(0.)))
                .child(div().w(viewport.width).h(viewport.height).children(slots)),
        )
        .with_priority(DIALOG_PRIORITY)
        .into_any_element()
    }
}

/// One dialog of the stack, built while the layer lays out, after its owner staged it.
#[derive(IntoElement)]
struct Slot {
    layer: Entity<DialogLayer>,
    key: Key,
    builder: Option<Builder>,
    index: usize,
    topmost: bool,
    dimmed: bool,
}

struct Shown {
    popup: FocusHandle,
    fade: Fade,
    closing: bool,
    generation: u64,
    needs_focus: bool,
}

impl RenderOnce for Slot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.key.clone();
        let dialog = match &self.builder {
            Some(build) => Some(build(Dialog::new(key.element_id()), window, cx)),
            None => self.layer.update(cx, |layer, _| layer.staged.remove(&key)),
        };
        let Some(dialog) = dialog else {
            return Empty.into_any_element();
        };
        let shown = self.layer.update(cx, |layer, cx| {
            let entry = layer.entry_mut(&key)?;
            entry.inner_focus = window
                .focused(cx)
                .filter(|focused| entry.popup.contains(focused, window))
                .map(|focused| focused.downgrade());
            entry.final_focus = dialog.final_focus.clone();
            if entry.builder.is_some() {
                entry.on_open_change = dialog.on_open_change.clone();
            }
            Some(Shown {
                popup: entry.popup.clone(),
                fade: entry.fade,
                closing: entry.closing,
                generation: entry.generation,
                needs_focus: std::mem::take(&mut entry.needs_focus),
            })
        });
        let Some(shown) = shown else {
            return Empty.into_any_element();
        };
        let now = cx.background_executor().now();
        let duration = DialogLayer::duration(cx);
        if !shown.fade.is_settled(now, duration) {
            window.request_animation_frame();
        }
        if shown.needs_focus && !shown.closing {
            let layer = self.layer.downgrade();
            let key = key.clone();
            let popup = shown.popup.clone();
            let initial = dialog.initial_focus.clone();
            let generation = shown.generation;
            window.defer(cx, move |window, cx| {
                let current = layer.upgrade().is_some_and(|layer| {
                    layer
                        .read(cx)
                        .entry(&key)
                        .is_some_and(|entry| !entry.closing && entry.generation == generation)
                });
                if current {
                    focus_initial(&popup, &initial, window, cx);
                }
            });
        }
        let visibility = shown.fade.value(now, duration);
        self.render_entry(dialog, shown, visibility, window, cx)
    }
}

impl Slot {
    fn render_entry(
        self,
        dialog: Dialog,
        shown: Shown,
        visibility: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let viewport = window.viewport_size();
        let interactive = !shown.closing;
        let alert = dialog.alert;
        let pointer_dismissal = !alert && dialog.surface != Surface::Wizard;
        let shows_close = dialog.shows_close_button();
        let index = self.index;
        let topmost = self.topmost;

        let backdrop = {
            let layer = self.layer.clone();
            let key = self.key.clone();
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .when(self.dimmed, |this| {
                    this.bg(colors.backdrop.opacity(visibility))
                })
                .when(interactive, |this| {
                    this.on_any_mouse_down(move |event, window, cx| {
                        if popover_at(event.position, window, cx) {
                            return;
                        }
                        cx.stop_propagation();
                        if topmost
                            && pointer_dismissal
                            && event.button == MouseButton::Left
                            && !any_popover_open(window, cx)
                        {
                            DialogLayer::request_close(
                                &layer,
                                &key,
                                OpenChangeReason::OutsidePress,
                                window,
                                cx,
                            );
                        }
                    })
                })
        };

        let has_header = dialog.title.is_some() || dialog.description.is_some();
        let has_body = !dialog.body.is_empty();
        let page = dialog.surface == Surface::Page;
        let palette = dialog.surface == Surface::Palette;
        let framed = page || dialog.surface == Surface::Wizard;
        let wide = viewport.width >= px(1024.);
        let popup = div()
            .id("popup")
            .debug_selector(move || format!("hoshi-dialog-popup-{index}"))
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .map(|this| {
                if page {
                    this.h_full()
                        .max_w(px(1280.))
                        .max_h(px(960.))
                        .bg(colors.background)
                        .text_color(colors.foreground)
                } else {
                    this.max_w(dialog.size.max_width())
                        .max_h_full()
                        .bg(colors.popover)
                        .text_color(colors.popover_foreground)
                }
            })
            .text_sm()
            .border_1()
            .border_color(colors.foreground.opacity(0.1))
            .when(!page, |this| this.opacity(visibility))
            .role(if alert {
                Role::AlertDialog
            } else {
                Role::Dialog
            })
            .when_some(dialog.title.clone(), |this, title| this.aria_label(title))
            .when_some(dialog.description.clone(), |this, description| {
                this.aria_description(description)
            })
            .tab_group()
            .map(|this| match dialog.surface {
                Surface::Standard => this
                    .rounded(px(14.))
                    .when(has_header, |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .flex_col()
                                .gap_2()
                                .px_4()
                                .pt_4()
                                .when(!has_body, |this| this.pb_4())
                                .when(shows_close, |this| this.pr_10())
                                .when_some(dialog.title.clone(), |this, title| {
                                    this.child(
                                        div()
                                            .text_base()
                                            .line_height(relative(1.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(title),
                                    )
                                })
                                .when_some(dialog.description.clone(), |this, description| {
                                    this.child(
                                        div()
                                            .text_color(colors.muted_foreground)
                                            .child(description),
                                    )
                                }),
                        )
                    })
                    .when(has_body, |this| {
                        this.child(
                            div()
                                .id("body")
                                .flex_shrink(1.)
                                .min_h_0()
                                .overflow_y_scroll()
                                .child(div().flex().flex_col().gap_4().p_4().children(dialog.body)),
                        )
                    })
                    .when(!dialog.footer.is_empty(), |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .flex_row()
                                .justify_end()
                                .gap_2()
                                .p_4()
                                .border_t_1()
                                .border_color(colors.border)
                                .bg(colors.muted.opacity(0.5))
                                .rounded_b(px(13.))
                                .children(dialog.footer),
                        )
                    })
                    .when(shows_close, |this| {
                        this.child(
                            div().absolute().top_2().right_2().child(
                                DialogButton::new("dialog-close")
                                    .icon(IconName::X)
                                    .variant(DialogButtonVariant::Ghost)
                                    .size(DialogButtonSize::IconSm)
                                    .when_some(dialog.close_label.clone(), |this, label| {
                                        this.accessible_label(label)
                                    })
                                    .close(),
                            ),
                        )
                    }),
                Surface::Palette => this
                    .rounded(px(14.))
                    .shadow_2xl()
                    .overflow_hidden()
                    .children(dialog.body),
                Surface::Page | Surface::Wizard => this
                    .rounded(px(16.))
                    .shadow_2xl()
                    .overflow_hidden()
                    .children(dialog.body),
                Surface::Confirm => this.rounded(px(18.)).shadow_2xl().child(
                    div()
                        .id("body")
                        .flex_shrink(1.)
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(div().flex().flex_col().p_5().children(dialog.body)),
                ),
            })
            .when(interactive, |this| {
                let escape = (self.layer.clone(), self.key.clone());
                let close = (self.layer.clone(), self.key.clone());
                this.occlude()
                    .key_context(KEY_CONTEXT)
                    .on_action(move |_: &Dismiss, window, cx| {
                        DialogLayer::request_close(
                            &escape.0,
                            &escape.1,
                            OpenChangeReason::EscapeKey,
                            window,
                            cx,
                        )
                    })
                    .on_action(move |_: &Close, window, cx| {
                        DialogLayer::request_close(
                            &close.0,
                            &close.1,
                            OpenChangeReason::ClosePress,
                            window,
                            cx,
                        )
                    })
            });
        let popup = if interactive {
            popup.focus_trap("trap", &shown.popup).into_any_element()
        } else {
            popup.into_any_element()
        };

        div()
            .id(self.key.element_id())
            .absolute()
            .top_0()
            .left_0()
            .w(viewport.width)
            .h(viewport.height)
            .when(interactive, |this| this.occlude())
            .child(backdrop)
            .when(framed, |this| {
                this.child(
                    div()
                        .id("page-drag")
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h_10()
                        .drag_region(),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .map(|this| match (framed, wide && page) {
                        (false, _) => this.p_4(),
                        (true, false) => this.px_6().pb_6().pt_12(),
                        (true, true) => this.px_10().pb_10().pt_12(),
                    })
                    .flex()
                    .map(|this| {
                        if palette {
                            this.items_start().pt(viewport.height * 0.12)
                        } else {
                            this.items_center()
                        }
                    })
                    .justify_center()
                    .child(popup),
            )
            .into_any_element()
    }
}

fn focus_initial(popup: &FocusHandle, initial: &InitialFocus, window: &mut Window, cx: &mut App) {
    match initial {
        InitialFocus::Handle(handle) => handle.focus(window, cx),
        InitialFocus::Popup => popup.focus(window, cx),
        InitialFocus::FirstTabbable => {
            popup.focus(window, cx);
            window.focus_next(cx);
            if !popup.contains_focused(window, cx) {
                popup.focus(window, cx);
            }
        }
    }
}
