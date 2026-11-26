//! One thread's shell on screen: the engine's output drawn by the emulator, keys, pastes and
//! the mouse sent back. Dropping the view leaves the shell running; `detach` stops the
//! streaming.

use super::colors::Palette;
use super::emulator::{Emulator, GridSize, Link};
use super::frame::{CursorLook, CursorState, Frame};
use super::{keys, mouse};
use crate::edit_menu::{EditAction, EditMenu, EditTarget, RunEdit, edit_items, edit_menu};
use crate::m;
use crate::state::Workspace;
use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::term::{TermMode, point_to_viewport};
use alacritty_terminal::vte::ansi::Rgb;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{CommandError, TerminalAttach, TerminalEvent, TerminalExit};
use hoshi_engine::Engine;
use hoshi_ui::prelude::*;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const LINE_HEIGHT: f32 = 1.2;
/// xterm.js's blink interval.
const BLINK: Duration = Duration::from_millis(600);
const FIRST_SIZE: GridSize = GridSize {
    columns: 80,
    lines: 24,
};

#[derive(Clone, Debug, PartialEq)]
enum Shell {
    /// Not laid out yet: the shell starts at the size the view first gets.
    Waiting,
    Starting,
    Running,
    Exited,
    Failed(String),
}

enum Input {
    Write(String),
    Resize(GridSize),
}

/// The shell ended, with the code it exited with when the platform reported one.
pub struct ShellExited {
    pub code: Option<u32>,
}

impl EventEmitter<ShellExited> for TerminalView {}

/// Opens another terminal, which starts where the thread works now.
pub type OpenHere = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    cell_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
}

/// What a frame paints besides the grid.
struct Overlay {
    link: Option<Vec<(usize, Range<usize>)>>,
    /// The input method's text and the cell it starts at.
    preedit: Option<(String, usize, usize)>,
}

pub struct TerminalView {
    workspace: Entity<Workspace>,
    engine: Arc<Engine>,
    thread_id: String,
    terminal_id: String,
    /// Run instead of an interactive shell, outside any thread.
    command: Option<String>,
    on_open_here: OpenHere,
    emulator: Emulator,
    shell: Shell,
    started_in: Option<String>,
    focus: FocusHandle,
    input: UnboundedSender<Input>,
    metrics: Option<Metrics>,
    grid_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    selecting: bool,
    scrolled: Pixels,
    detached: bool,
    pump: Option<Task<()>>,
    /// The button a program that reads the mouse saw go down, until it comes up.
    reported_button: Option<mouse::Button>,
    reported_cell: Option<(usize, usize)>,
    /// The address under the pointer.
    link: Option<Link>,
    pressed_link: Option<String>,
    /// What the input method is composing, shown at the cursor until it commits.
    marked: Option<String>,
    focused: bool,
    blink_on: bool,
    blink: Option<Task<()>>,
    sync_timer: Option<Task<()>>,
    /// The right-click menu, where it opened, and which opening it is.
    menu: Option<(Point<Pixels>, usize)>,
    menus_opened: usize,
    #[cfg(test)]
    sent: std::cell::RefCell<Vec<String>>,
    _writer: Task<()>,
    _keys: Subscription,
}

impl TerminalView {
    pub fn new(
        workspace: Entity<Workspace>,
        thread_id: String,
        terminal_id: String,
        on_open_here: OpenHere,
        cx: &mut Context<Self>,
    ) -> Self {
        let engine = workspace.read(cx).engine.clone();
        let (input, mut inputs) = unbounded::<Input>();
        let writer = {
            let engine = engine.clone();
            let terminal_id = terminal_id.clone();
            cx.background_spawn(async move {
                while let Some(input) = inputs.next().await {
                    let _ = match input {
                        Input::Write(data) => engine.write_terminal(&terminal_id, &data),
                        Input::Resize(size) => {
                            engine.resize_terminal(&terminal_id, engine_size(size))
                        }
                    };
                }
            })
        };
        let focus = cx.focus_handle();
        let keys = {
            let focus = focus.clone();
            let this = cx.entity().downgrade();
            cx.intercept_keystrokes(move |event, window, cx| {
                if !focus.is_focused(window) || keys::is_app_shortcut(&event.keystroke) {
                    return;
                }
                let handled = this
                    .update(cx, |view, cx| view.key(&event.keystroke, cx))
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            })
        };
        Self {
            workspace,
            engine,
            thread_id,
            terminal_id,
            command: None,
            on_open_here,
            emulator: Emulator::new(FIRST_SIZE),
            shell: Shell::Waiting,
            started_in: None,
            focus,
            input,
            metrics: None,
            grid_bounds: Rc::default(),
            selecting: false,
            scrolled: px(0.),
            detached: false,
            pump: None,
            reported_button: None,
            reported_cell: None,
            link: None,
            pressed_link: None,
            marked: None,
            focused: false,
            blink_on: true,
            blink: None,
            sync_timer: None,
            menu: None,
            menus_opened: 0,
            #[cfg(test)]
            sent: Default::default(),
            _writer: writer,
            _keys: keys,
        }
    }

    /// A shell in the home folder that runs `command` and exits with it.
    pub fn run(
        workspace: Entity<Workspace>,
        terminal_id: String,
        command: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(
            workspace,
            String::new(),
            terminal_id,
            Rc::new(|_, _| {}),
            cx,
        );
        view.command = Some(command);
        view
    }

    #[cfg(test)]
    pub fn started_in(&self) -> Option<&str> {
        self.started_in.as_deref()
    }

    pub fn terminal_id(&self) -> &str {
        &self.terminal_id
    }

    /// The visible rows as text, for tests and debugging.
    #[cfg(test)]
    pub fn screen_text(&self) -> Vec<String> {
        let palette = Palette::new(&hoshi_ui::theme::colors(hoshi_ui::theme::ThemeMode::Dark));
        let frame = Frame::read(self.emulator.term(), &palette, CursorState::Blurred);
        (0..frame.rows.len())
            .map(|line| frame.row_text(line))
            .collect()
    }

    #[cfg(test)]
    pub fn grid_size(&self) -> GridSize {
        self.emulator.size()
    }

    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.shell == Shell::Running
    }

    #[cfg(test)]
    pub fn hovered_link(&self) -> Option<&str> {
        self.link.as_ref().map(|link| link.url.as_str())
    }

    #[cfg(test)]
    pub fn cursor_shown(&self) -> bool {
        self.blink_on
    }

    #[cfg(test)]
    pub fn preedit(&self) -> Option<&str> {
        self.marked.as_deref()
    }

    /// Output as if the shell printed it.
    #[cfg(test)]
    pub fn feed(&mut self, data: &str, cx: &mut Context<Self>) {
        self.receive(
            vec![TerminalEvent::Output {
                data: data.to_owned(),
            }],
            cx,
        );
    }

    /// What the view sent the shell since the last call.
    #[cfg(test)]
    pub fn take_sent(&self) -> Vec<String> {
        std::mem::take(&mut *self.sent.borrow_mut())
    }

    #[cfg(test)]
    pub fn grid_origin(&self) -> Option<(Point<Pixels>, Pixels, Pixels)> {
        let bounds = self.grid_bounds.get()?;
        let metrics = self.metrics?;
        Some((bounds.origin, metrics.cell_width, metrics.line_height))
    }

    /// Stops streaming to this view; the shell keeps running.
    pub fn detach(&mut self) {
        if self.detached {
            return;
        }
        self.detached = true;
        self.pump = None;
        self.blink = None;
        self.sync_timer = None;
        self.engine.detach_terminal(&self.terminal_id);
    }

    fn start(&mut self, size: GridSize, restart: bool, cx: &mut Context<Self>) {
        if self.detached {
            return;
        }
        self.shell = Shell::Starting;
        let (sender, mut events) = unbounded::<TerminalEvent>();
        let engine = self.engine.clone();
        let thread_id = self.thread_id.clone();
        let terminal_id = self.terminal_id.clone();
        let command = self.command.clone();
        let draft_project = self.workspace.read(cx).draft_project(&thread_id);
        self.pump = Some(cx.spawn(async move |this, cx| {
            let attach = cx
                .background_spawn(async move {
                    if restart {
                        engine.close_terminal(&terminal_id);
                    }
                    let sink: hoshi_engine::TerminalSink = Box::new(move |event| {
                        let _ = sender.unbounded_send(event);
                    });
                    let size = engine_size(size);
                    match (command, draft_project) {
                        (Some(command), _) => {
                            engine.run_in_terminal(&terminal_id, &command, size, sink)
                        }
                        (None, Some(project_id)) => engine.open_draft_terminal(
                            &thread_id,
                            project_id.as_deref(),
                            &terminal_id,
                            size,
                            sink,
                        ),
                        (None, None) => engine.open_terminal(&thread_id, &terminal_id, size, sink),
                    }
                })
                .await;
            if this
                .update(cx, |view, cx| view.attached(attach, cx))
                .is_err()
            {
                return;
            }
            while let Some(first) = events.next().await {
                let mut batch = vec![first];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |view, cx| view.receive(batch, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn attached(&mut self, attach: Result<TerminalAttach, CommandError>, cx: &mut Context<Self>) {
        match attach {
            Ok(attach) => {
                self.started_in = Some(attach.cwd);
                self.shell = Shell::Running;
                self.emulator.feed(attach.history.as_bytes());
                self.emulator.take_events();
                if let Some(exit) = attach.exit {
                    self.exited(exit, cx);
                }
                self.schedule_sync(cx);
            }
            Err(CommandError::TerminalFailed { detail }) => self.shell = Shell::Failed(detail),
            Err(_) => self.shell = Shell::Failed(m::error_generic()),
        }
        cx.notify();
    }

    fn receive(&mut self, batch: Vec<TerminalEvent>, cx: &mut Context<Self>) {
        for event in batch {
            match event {
                TerminalEvent::Output { data } => self.emulator.feed(data.as_bytes()),
                TerminalEvent::Exited { exit } => self.exited(exit, cx),
            }
        }
        self.answer(cx);
        self.schedule_sync(cx);
        self.link = None;
        self.restart_blink(cx);
        cx.notify();
    }

    /// Draws a synchronized update the program never ended once its time is up.
    fn schedule_sync(&mut self, cx: &mut Context<Self>) {
        let Some(deadline) = self.emulator.sync_deadline() else {
            self.sync_timer = None;
            return;
        };
        if self.sync_timer.is_some() || self.detached {
            return;
        }
        self.sync_timer = Some(cx.spawn(async move |this, cx| {
            let wait = deadline.saturating_duration_since(Instant::now());
            cx.background_executor().timer(wait).await;
            this.update(cx, |view, cx| {
                view.sync_timer = None;
                match view.emulator.sync_deadline() {
                    Some(due) if due <= Instant::now() => {
                        view.emulator.end_sync();
                        view.answer(cx);
                        view.schedule_sync(cx);
                        cx.notify();
                    }
                    Some(_) => view.schedule_sync(cx),
                    None => {}
                }
            })
            .ok();
        }));
    }

    /// Shows the cursor and blinks it from there while the view has the keyboard.
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_on = true;
        if !self.focused || self.detached || cx.reduce_motion() {
            self.blink = None;
            return;
        }
        self.blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                let alive = this.update(cx, |view, cx| {
                    let shown =
                        !view.blink_on || !view.emulator.cursor_blinks() || cx.reduce_motion();
                    if shown != view.blink_on {
                        view.blink_on = shown;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        if self.shell == Shell::Running && self.emulator.mode().contains(TermMode::FOCUS_IN_OUT) {
            self.write(if focused { "\x1b[I" } else { "\x1b[O" }.to_owned());
        }
        self.restart_blink(cx);
    }

    fn exited(&mut self, exit: TerminalExit, cx: &mut Context<Self>) {
        self.shell = Shell::Exited;
        cx.emit(ShellExited { code: exit.code });
        let notice = match exit.code {
            Some(code) => m::terminal_exited_with_code(code),
            None => m::terminal_exited(),
        };
        self.emulator
            .feed(format!("\r\n\x1b[2m{notice}\x1b[0m\r\n").as_bytes());
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        let size = self.emulator.size();
        self.emulator = Emulator::new(size);
        self.start(size, true, cx);
        cx.notify();
    }

    /// Sends the shell what the emulation answers to its queries.
    fn answer(&mut self, cx: &mut Context<Self>) {
        for event in self.emulator.take_events() {
            match event {
                Event::PtyWrite(text) => self.write(text),
                Event::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Event::ColorRequest(index, format) => {
                    let palette = Palette::new(cx.hoshi());
                    let color = match index {
                        0..=255 => palette.indexed(index as u8),
                        256 => palette.foreground,
                        257 => palette.background,
                        258 => palette.cursor,
                        _ => palette.foreground,
                    };
                    self.write(format(to_rgb(color)));
                }
                Event::TextAreaSizeRequest(format) => {
                    let size = self.emulator.size();
                    let (cell_width, cell_height) = self.metrics.map_or((0, 0), |metrics| {
                        (
                            f32::from(metrics.cell_width) as u16,
                            f32::from(metrics.line_height) as u16,
                        )
                    });
                    self.write(format(WindowSize {
                        num_lines: size.lines as u16,
                        num_cols: size.columns as u16,
                        cell_width,
                        cell_height,
                    }));
                }
                _ => {}
            }
        }
    }

    fn write(&self, data: String) {
        #[cfg(test)]
        self.sent.borrow_mut().push(data.clone());
        let _ = self.input.unbounded_send(Input::Write(data));
    }

    /// What the user typed: the view follows the prompt again.
    fn send(&mut self, data: String, cx: &mut Context<Self>) {
        if self.shell == Shell::Exited {
            self.restart(cx);
            return;
        }
        self.emulator.scroll_to_bottom();
        self.emulator.clear_selection();
        self.write(data);
        self.restart_blink(cx);
        cx.notify();
    }

    fn key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let modifiers = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        let plain_ctrl = modifiers.control && !modifiers.alt && !modifiers.platform;
        let selected = self.emulator.selected_text();
        if plain_ctrl && key == "c" && (modifiers.shift || selected.is_some()) {
            if let Some(text) = selected {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            self.emulator.clear_selection();
            cx.notify();
            return true;
        }
        if plain_ctrl && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.paste(&text, cx);
            }
            return true;
        }
        if self.shell == Shell::Exited && !modifiers.platform {
            self.restart(cx);
            return true;
        }
        let app_cursor = self.emulator.mode().contains(TermMode::APP_CURSOR);
        match keys::key_bytes(keystroke, app_cursor) {
            Some(bytes) => {
                self.send(bytes, cx);
                true
            }
            None => false,
        }
    }

    fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.emulator.mode().contains(TermMode::BRACKETED_PASTE);
        self.send(keys::paste_bytes(text, bracketed), cx);
    }

    fn fit(&mut self, size: GridSize, cx: &mut Context<Self>) {
        if self.shell == Shell::Waiting {
            self.emulator.resize(size);
            self.start(size, false, cx);
        } else if self.emulator.resize(size) {
            let _ = self.input.unbounded_send(Input::Resize(size));
        }
        cx.notify();
    }

    /// The cell under a point, held to the grid, and whether the point is in its right half.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, bool)> {
        let bounds = self.grid_bounds.get()?;
        let metrics = self.metrics?;
        let size = self.emulator.size();
        let x = f32::from(position.x - bounds.origin.x).max(0.) / f32::from(metrics.cell_width);
        let y = f32::from(position.y - bounds.origin.y).max(0.) / f32::from(metrics.line_height);
        Some((
            (y as usize).min(size.lines.saturating_sub(1)),
            (x as usize).min(size.columns.saturating_sub(1)),
            x.fract() >= 0.5,
        ))
    }

    fn link_under(&self, position: Point<Pixels>) -> Option<Link> {
        let over = self
            .grid_bounds
            .get()
            .is_some_and(|bounds| bounds.contains(&position));
        let (line, column, _) = self.cell_at(position).filter(|_| over)?;
        if let Some(link) = self.link.as_ref().filter(|link| link.covers(line, column)) {
            return Some(link.clone());
        }
        self.emulator.link_at(line, column)
    }

    /// Tells a program that reads the mouse what it did; `false` when the mouse is the
    /// user's, to select text and open links.
    fn report(
        &mut self,
        button: mouse::Button,
        action: mouse::Action,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) -> bool {
        let mode = self.emulator.mode();
        if !mouse::reporting(mode, modifiers.shift) {
            return false;
        }
        let Some((line, column, _)) = self.cell_at(position) else {
            return true;
        };
        if action == mouse::Action::Motion && self.reported_cell == Some((line, column)) {
            return true;
        }
        self.reported_cell = Some((line, column));
        let held = mouse::Modifiers {
            shift: modifiers.shift,
            alt: modifiers.alt,
            control: modifiers.control,
        };
        if let Some(data) = mouse::report(mode, button, action, line, column, held) {
            self.write(data);
        }
        true
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let button = match event.button {
            MouseButton::Left => mouse::Button::Left,
            MouseButton::Middle => mouse::Button::Middle,
            MouseButton::Right => mouse::Button::Right,
            _ => return,
        };
        if self.report(
            button,
            mouse::Action::Press,
            event.position,
            event.modifiers,
        ) {
            self.reported_button = Some(button);
            return;
        }
        if button == mouse::Button::Right {
            self.menus_opened += 1;
            self.menu = Some((event.position, self.menus_opened));
            cx.notify();
            return;
        }
        if button != mouse::Button::Left {
            return;
        }
        let Some((line, column, right)) = self.cell_at(event.position) else {
            return;
        };
        self.pressed_link = self.link_under(event.position).map(|link| link.url);
        if event.modifiers.shift && self.emulator.selected_text().is_some() {
            self.emulator.select_to(line, column, right);
        } else if event.click_count > 1 {
            self.emulator
                .select_from(line, column, right, event.click_count);
        } else {
            self.emulator.clear_selection();
            self.emulator.select_from(line, column, right, 1);
        }
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(button) = self.reported_button {
            self.report(
                button,
                mouse::Action::Motion,
                event.position,
                event.modifiers,
            );
            return;
        }
        if mouse::reporting(self.emulator.mode(), event.modifiers.shift) {
            self.report(
                mouse::Button::None,
                mouse::Action::Motion,
                event.position,
                event.modifiers,
            );
            self.set_link(None, cx);
            return;
        }
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            if let Some((line, column, right)) = self.cell_at(event.position) {
                self.emulator.select_to(line, column, right);
                cx.notify();
            }
            return;
        }
        let link = self.link_under(event.position);
        self.set_link(link, cx);
    }

    fn set_link(&mut self, link: Option<Link>, cx: &mut Context<Self>) {
        if self.link != link {
            self.link = link;
            cx.notify();
        }
    }

    fn edit_menu(
        &self,
        position: Point<Pixels>,
        serial: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let selected = self.emulator.selected_text().is_some();
        let enabled = edit_menu(EditTarget::Terminal, selected).unwrap_or(EditMenu {
            cut: false,
            copy: false,
            paste: true,
            select_all: true,
        });
        let closing = cx.weak_entity();
        let on_close = move |_: &mut Window, cx: &mut App| {
            closing
                .update(cx, |this, cx| {
                    if this.menu.map(|(_, open)| open) == Some(serial) {
                        this.menu = None;
                        cx.notify();
                    }
                })
                .ok();
        };
        let view = cx.weak_entity();
        let run: RunEdit = Rc::new(move |action, window, cx| {
            view.update(cx, |this, cx| this.edit(action, window, cx))
                .ok();
        });
        context_menu(
            SharedString::from(format!("terminal-menu-{}-{serial}", self.terminal_id)),
            position,
            on_close,
            move |menu, _, _| edit_items(menu, enabled, run),
        )
    }

    fn edit(&mut self, action: EditAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            EditAction::Copy => {
                if let Some(text) = self.emulator.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            EditAction::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.paste(&text, cx);
                }
            }
            EditAction::SelectAll => {
                self.emulator.select_all();
                cx.notify();
            }
            EditAction::Cut => {}
        }
        self.focus.focus(window, cx);
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(button) = self.reported_button.take() {
            self.report(
                button,
                mouse::Action::Release,
                event.position,
                event.modifiers,
            );
            self.reported_cell = None;
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;
        let pressed = self.pressed_link.take();
        if self.emulator.selected_text().is_none() {
            self.emulator.clear_selection();
            let released = self.link_under(event.position);
            if let (Some(pressed), Some(released)) = (pressed, released)
                && pressed == released.url
            {
                let external = event.modifiers.control || event.modifiers.platform;
                crate::markdown::open_web_link(&pressed, external, cx);
            }
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics else {
            return;
        };
        self.scrolled += event.delta.pixel_delta(metrics.line_height).y;
        let lines = (self.scrolled / metrics.line_height) as i32;
        if lines == 0 {
            return;
        }
        self.scrolled -= metrics.line_height * lines as f32;
        cx.stop_propagation();
        let mode = self.emulator.mode();
        if mouse::reporting(mode, event.modifiers.shift) {
            let wheel = if lines > 0 {
                mouse::Button::WheelUp
            } else {
                mouse::Button::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.reported_cell = None;
                self.report(wheel, mouse::Action::Press, event.position, event.modifiers);
            }
            return;
        }
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            let arrow = match (lines > 0, app_cursor) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.write(arrow.repeat(lines.unsigned_abs() as usize));
            return;
        }
        self.emulator.scroll_lines(lines);
        self.link = None;
        cx.notify();
    }

    fn measure(window: &mut Window, cx: &App) -> Metrics {
        let (family, font_size) = hoshi_ui::theme::terminal_font(cx);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font(family));
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map(|advance| advance.width)
            .unwrap_or(font_size * 0.6);
        Metrics {
            cell_width,
            line_height: (font_size * LINE_HEIGHT).round(),
            font_size,
        }
    }

    /// The folder the thread works in now, when the shell started somewhere else.
    fn moved_to(&self, cx: &App) -> Option<String> {
        let started_in = self.started_in.as_deref()?;
        let now = self.workspace.read(cx).model.thread_cwd(&self.thread_id)?;
        (started_in != now).then(|| {
            std::path::Path::new(now)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| now.to_owned())
        })
    }

    fn cursor_cell(&self) -> Option<(usize, usize)> {
        let term = self.emulator.term();
        let point = point_to_viewport(term.grid().display_offset(), term.grid().cursor.point)?;
        Some((point.line, point.column.0))
    }
}

fn engine_size(size: GridSize) -> (u16, u16) {
    (
        size.columns.min(usize::from(u16::MAX)) as u16,
        size.lines.min(usize::from(u16::MAX)) as u16,
    )
}

fn to_rgb(color: Hsla) -> Rgb {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    Rgb {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
    }
}

fn moved_bar(folder: String, on_open_here: OpenHere, colors: &Colors) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        .border_b_1()
        .border_color(colors.border)
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(Icon::new(IconName::GitFork).size_3p5().flex_none())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(m::terminal_thread_moved(folder)),
        )
        .child(
            Button::new("terminal-open-here")
                .label(m::open_terminal_here())
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Outline)
                .on_click(move |_, window, cx| on_open_here(window, cx)),
        )
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let palette = Palette::new(&colors);
        let metrics = Self::measure(window, cx);
        self.metrics = Some(metrics);
        let focused = self.focus.is_focused(window);
        self.set_focused(focused, cx);
        let cursor = if !focused {
            CursorState::Blurred
        } else if self.blink_on || self.marked.is_some() {
            CursorState::Focused
        } else {
            CursorState::BlinkedOff
        };
        let frame = Frame::read(self.emulator.term(), &palette, cursor);
        let overlay = Overlay {
            link: self.link.as_ref().map(|link| link.spans.clone()),
            preedit: self.marked.clone().and_then(|text| {
                let (line, column) = self.cursor_cell()?;
                Some((text, line, column))
            }),
        };
        let size = self.emulator.size();
        let waiting = self.shell == Shell::Waiting;
        let error = match &self.shell {
            Shell::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let moved_to = self.moved_to(cx);
        let reporting = self.emulator.mode().intersects(TermMode::MOUSE_MODE);
        let pointer = if self.link.is_some() {
            CursorStyle::PointingHand
        } else if reporting {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        };
        let menu = self
            .menu
            .map(|(position, serial)| self.edit_menu(position, serial, cx));
        let (family, _) = hoshi_ui::theme::terminal_font(cx);
        let this = cx.entity();
        let weak = this.downgrade();
        let grid_bounds = self.grid_bounds.clone();
        let focus = self.focus.clone();

        div()
            .id(SharedString::from(format!("terminal-{}", self.terminal_id)))
            .track_focus(&self.focus)
            .key_context("Terminal")
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .when_some(moved_to, |this, folder| {
                this.child(moved_bar(folder, self.on_open_here.clone(), &colors))
            })
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(colors.destructive)
                        .child(error),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!(
                        "terminal-grid-{}",
                        self.terminal_id
                    )))
                    .flex_1()
                    .min_h_0()
                    .py_1()
                    .pl_2()
                    .cursor(pointer)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, window, cx| this.mouse_down(event, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(|this, event, window, cx| this.mouse_down(event, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event, window, cx| this.mouse_down(event, window, cx)),
                    )
                    .on_mouse_move(cx.listener(|this, event, _, cx| this.mouse_move(event, cx)))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Middle,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, event, _, cx| this.mouse_up(event, cx)),
                    )
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered {
                            this.set_link(None, cx);
                        }
                    }))
                    .on_scroll(cx.listener(|this, event, _, cx| this.scroll(event, cx)))
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                grid_bounds.set(Some(bounds));
                                let fitted = GridSize {
                                    columns: (f32::from(bounds.size.width)
                                        / f32::from(metrics.cell_width))
                                    .floor()
                                    .max(2.) as usize,
                                    lines: (f32::from(bounds.size.height)
                                        / f32::from(metrics.line_height))
                                    .floor()
                                    .max(1.) as usize,
                                };
                                if waiting || fitted != size {
                                    cx.defer(move |cx| {
                                        weak.update(cx, |view, cx| view.fit(fitted, cx)).ok();
                                    });
                                }
                            },
                            move |bounds, _, window, cx| {
                                paint(&frame, &overlay, bounds, metrics, family, window, cx);
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, this),
                                    cx,
                                );
                            },
                        )
                        .size_full(),
                    ),
            )
            .children(menu)
    }
}

fn paint(
    frame: &Frame,
    overlay: &Overlay,
    bounds: Bounds<Pixels>,
    metrics: Metrics,
    family: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let cell = |line: usize, column: usize, width: usize| {
        Bounds::new(
            point(
                bounds.origin.x + metrics.cell_width * column as f32,
                bounds.origin.y + metrics.line_height * line as f32,
            ),
            size(metrics.cell_width * width as f32, metrics.line_height),
        )
    };
    let thickness = px(1.);
    for (line, row) in frame.rows.iter().enumerate() {
        for run in row.fills.iter().chain(&row.highlights) {
            window.paint_quad(fill(cell(line, run.column, run.width), run.color));
        }
    }
    if let Some(cursor) = frame.cursor {
        let at = cell(cursor.line, cursor.column, cursor.width);
        let thickness = px(2.);
        match cursor.look {
            CursorLook::Block => {}
            CursorLook::Hollow => {
                window.paint_quad(outline(at, cursor.color, BorderStyle::Solid));
            }
            CursorLook::Beam => {
                window.paint_quad(fill(
                    Bounds::new(at.origin, size(thickness, at.size.height)),
                    cursor.color,
                ));
            }
            CursorLook::Underline => {
                window.paint_quad(fill(
                    Bounds::new(
                        point(at.origin.x, at.origin.y + at.size.height - thickness),
                        size(at.size.width, thickness),
                    ),
                    cursor.color,
                ));
            }
        }
    }
    let mono = |bold: bool, italic: bool| Font {
        family: family.clone(),
        weight: if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        ..font(family.clone())
    };
    for (line, row) in frame.rows.iter().enumerate() {
        for text in &row.texts {
            let run = TextRun {
                len: text.text.len(),
                font: mono(text.bold, text.italic),
                color: text.color,
                background_color: None,
                underline: text.underline.then_some(UnderlineStyle {
                    thickness,
                    color: Some(text.color),
                    wavy: false,
                }),
                strikethrough: text.strike.then_some(StrikethroughStyle {
                    thickness,
                    color: Some(text.color),
                }),
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(text.text.clone()),
                metrics.font_size,
                &[run],
                None,
            );
            let origin = cell(line, text.column, 1).origin;
            shaped
                .paint(
                    origin,
                    metrics.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
        }
    }
    let foreground = cx.hoshi().foreground;
    for (line, columns) in overlay.link.iter().flatten() {
        let at = cell(*line, columns.start, columns.len());
        window.paint_quad(fill(
            Bounds::new(
                point(at.origin.x, at.origin.y + at.size.height - thickness),
                size(at.size.width, thickness),
            ),
            foreground,
        ));
    }
    if let Some((text, line, column)) = &overlay.preedit {
        let run = TextRun {
            len: text.len(),
            font: mono(false, false),
            color: foreground,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness,
                color: Some(foreground),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(text.clone()),
            metrics.font_size,
            &[run],
            None,
        );
        let origin = cell(*line, *column, 1).origin;
        window.paint_quad(fill(
            Bounds::new(origin, size(shaped.width, metrics.line_height)),
            cx.hoshi().background,
        ));
        shaped
            .paint(
                origin,
                metrics.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked: Vec<u16> = self.marked.as_ref()?.encode_utf16().collect();
        let range = range.start.min(marked.len())..range.end.min(marked.len());
        *adjusted = Some(range.clone());
        String::from_utf16(&marked[range]).ok()
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .marked
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            TerminalView::paste(self, &text, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_marked = self.marked.take().is_some();
        if !text.is_empty() {
            self.send(text.to_owned(), cx);
        } else if was_marked {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        self.restart_blink(cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let metrics = self.metrics?;
        let (line, column) = self.cursor_cell().unwrap_or((0, 0));
        Some(Bounds::new(
            point(
                element_bounds.origin.x + metrics.cell_width * column as f32,
                element_bounds.origin.y + metrics.line_height * line as f32,
            ),
            size(metrics.cell_width, metrics.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
