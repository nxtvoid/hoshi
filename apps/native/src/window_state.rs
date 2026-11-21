//! Where and how big the window was, kept in `window.json` under the home so the next start
//! opens it the same way.

use gpui_kit::{
    App, Bounds, Context, DisplayId, Entity, Pixels, Task, Window, WindowBounds, point, px, size,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const DEFAULT_SIZE: (f32, f32) = (1280., 820.);
const MIN_SIZE: f32 = 200.;
/// How much of the window's top edge, down and across, must be on a display to keep the saved
/// position: enough to grab it and drag it back.
const GRIP: (f32, f32) = (32., 100.);

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub bounds: WindowBounds,
    pub display: Option<String>,
}

/// A connected display, as `place` sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub uuid: Option<String>,
    pub bounds: Bounds<Pixels>,
    pub visible: Bounds<Pixels>,
}

pub fn file(home: &Path) -> PathBuf {
    home.join("window.json")
}

pub fn parse(text: &str) -> Option<Saved> {
    let value: Value = serde_json::from_str(text).ok()?;
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .map(|n| n as f32)
            .filter(|n| n.is_finite())
    };
    let (x, y) = (number("x")?, number("y")?);
    let (width, height) = (number("width")?, number("height")?);
    if width < MIN_SIZE || height < MIN_SIZE {
        return None;
    }
    let rect = Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    };
    let bounds = match value.get("state").and_then(Value::as_str) {
        Some("maximized") => WindowBounds::Maximized(rect),
        Some("fullscreen") => WindowBounds::Fullscreen(rect),
        _ => WindowBounds::Windowed(rect),
    };
    Some(Saved {
        bounds,
        display: value
            .get("display")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub fn serialize(saved: &Saved) -> String {
    let (state, rect) = match saved.bounds {
        WindowBounds::Windowed(rect) => ("windowed", rect),
        WindowBounds::Maximized(rect) => ("maximized", rect),
        WindowBounds::Fullscreen(rect) => ("fullscreen", rect),
    };
    json!({
        "state": state,
        "x": f32::from(rect.origin.x),
        "y": f32::from(rect.origin.y),
        "width": f32::from(rect.size.width),
        "height": f32::from(rect.size.height),
        "display": saved.display,
    })
    .to_string()
}

pub fn load(home: &Path) -> Option<Saved> {
    parse(&std::fs::read_to_string(file(home)).ok()?)
}

/// The bounds to reopen the window with and the index of the screen they go on. The saved
/// screen while it is connected, else the primary one, else the first. The size shrinks to fit
/// the screen; the position is kept while the window's top edge is on it, else it is centered.
pub fn place(
    saved: &Saved,
    screens: &[Screen],
    primary: Option<usize>,
) -> Option<(WindowBounds, usize)> {
    let index = saved
        .display
        .as_deref()
        .and_then(|uuid| {
            screens
                .iter()
                .position(|screen| screen.uuid.as_deref() == Some(uuid))
        })
        .or(primary)
        .or((!screens.is_empty()).then_some(0))?;
    let area = screens[index].visible;
    let rect = saved.bounds.get_bounds();
    let fitted = size(
        rect.size.width.min(area.size.width),
        rect.size.height.min(area.size.height),
    );
    let top = rect.origin.y;
    let across = (rect.origin.x + fitted.width).min(area.origin.x + area.size.width)
        - rect.origin.x.max(area.origin.x);
    let grabbable = top >= area.origin.y
        && top + px(GRIP.0) <= area.origin.y + area.size.height
        && across >= fitted.width.min(px(GRIP.1));
    let origin = if grabbable {
        rect.origin
    } else {
        let center = area.center();
        point(center.x - fitted.width / 2., center.y - fitted.height / 2.)
    };
    let rect = Bounds {
        origin,
        size: fitted,
    };
    let bounds = match saved.bounds {
        WindowBounds::Windowed(_) => WindowBounds::Windowed(rect),
        WindowBounds::Maximized(_) => WindowBounds::Maximized(rect),
        WindowBounds::Fullscreen(_) => WindowBounds::Fullscreen(rect),
    };
    Some((bounds, index))
}

/// The bounds and display to open the window with.
pub fn initial(home: &Path, cx: &App) -> (WindowBounds, Option<DisplayId>) {
    let displays = cx.displays();
    let screens: Vec<Screen> = displays
        .iter()
        .map(|display| Screen {
            uuid: display.uuid().ok().map(|uuid| uuid.to_string()),
            bounds: display.bounds(),
            visible: display.visible_bounds(),
        })
        .collect();
    let primary = cx.primary_display().and_then(|primary| {
        displays
            .iter()
            .position(|display| display.id() == primary.id())
    });
    match load(home).and_then(|saved| place(&saved, &screens, primary)) {
        Some((bounds, index)) => (bounds, Some(displays[index].id())),
        None => (
            WindowBounds::centered(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), cx),
            None,
        ),
    }
}

struct Keeper {
    home: PathBuf,
    written: Option<Saved>,
    saving: Option<Task<()>>,
}

impl Keeper {
    /// Writes the state unless it is already on disk; a failed write is tried again next time.
    fn save(&mut self, saved: Saved) {
        if self.written.as_ref() == Some(&saved) {
            return;
        }
        self.written = std::fs::write(file(&self.home), serialize(&saved))
            .is_ok()
            .then_some(saved);
    }
}

/// `None` while the window is minimized or hidden, when it reports a state it won't reopen in.
fn snapshot(window: &Window, cx: &App) -> Option<Saved> {
    window.is_visible().then(|| Saved {
        bounds: window.window_bounds(),
        display: window
            .display(cx)
            .and_then(|display| display.uuid().ok())
            .map(|uuid| uuid.to_string()),
    })
}

fn save_now(keeper: &RefCell<Keeper>, window: &Window, cx: &App) {
    let mut keeper = keeper.borrow_mut();
    keeper.saving = None;
    if let Some(saved) = snapshot(window, cx) {
        keeper.save(saved);
    }
}

/// Saves the window's bounds a moment after they stop changing, and right away when it closes.
pub fn remember<V: 'static>(home: PathBuf, view: &Entity<V>, window: &mut Window, cx: &mut App) {
    let keeper = Rc::new(RefCell::new(Keeper {
        written: load(&home),
        home,
        saving: None,
    }));
    view.update(cx, |_, cx: &mut Context<V>| {
        let schedule = {
            let keeper = keeper.clone();
            move |_: &mut V, window: &mut Window, cx: &mut Context<V>| {
                let pending = keeper.clone();
                let task = cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(300))
                        .await;
                    if let Ok(Some(saved)) = cx.update(|window, cx| snapshot(window, cx)) {
                        pending.borrow_mut().save(saved);
                    }
                });
                keeper.borrow_mut().saving = Some(task);
            }
        };
        cx.observe_window_bounds(window, schedule.clone()).detach();
        cx.observe_window_display(window, schedule).detach();
    });
    let on_close = keeper.clone();
    window.on_window_should_close(cx, move |window, cx| {
        save_now(&on_close, window, cx);
        true
    });
    let handle = window.window_handle();
    cx.on_app_quit(move |cx| {
        handle
            .update(cx, |_, window, cx| save_now(&keeper, window, cx))
            .ok();
        async {}
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{Keeper, Saved, Screen, file, parse, place, serialize};
    use gpui_kit::{Bounds, Pixels, WindowBounds, point, px, size};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    fn screen(uuid: &str, x: f32, width: f32, height: f32) -> Screen {
        Screen {
            uuid: Some(uuid.into()),
            bounds: rect(x, 0., width, height),
            visible: rect(x, 0., width, height - 40.),
        }
    }

    fn saved(bounds: WindowBounds, display: Option<&str>) -> Saved {
        Saved {
            bounds,
            display: display.map(str::to_owned),
        }
    }

    #[test]
    fn the_window_survives_a_round_trip_with_its_state() {
        for bounds in [
            WindowBounds::Windowed(rect(-1600., 40., 1200., 800.)),
            WindowBounds::Maximized(rect(100., 100., 1280., 820.)),
            WindowBounds::Fullscreen(rect(0., 0., 1440., 900.)),
        ] {
            let saved = saved(bounds, Some("4c4c4544-0000-0000-0000-000000000000"));
            assert_eq!(parse(&serialize(&saved)), Some(saved));
        }
        let saved = saved(WindowBounds::Windowed(rect(0., 0., 800., 600.)), None);
        assert_eq!(parse(&serialize(&saved)), Some(saved));
    }

    #[test]
    fn bad_tiny_or_endless_windows_read_as_nothing() {
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"x":0,"y":0,"width":800}"#), None);
        assert_eq!(parse(r#"{"x":0,"y":0,"width":0,"height":0}"#), None);
        assert_eq!(parse(r#"{"x":0,"y":0,"width":1e300,"height":800}"#), None);
    }

    #[test]
    fn the_window_goes_back_where_it_was_on_its_screen() {
        let screens = [
            screen("a", 0., 1920., 1080.),
            screen("b", 1920., 2560., 1440.),
        ];
        let was = WindowBounds::Maximized(rect(2100., 100., 1400., 900.));
        assert_eq!(
            place(&saved(was, Some("b")), &screens, Some(0)),
            Some((was, 1))
        );
    }

    #[test]
    fn a_window_off_its_screen_is_centered_there_keeping_size_and_state() {
        let screens = [
            screen("a", 0., 1920., 1080.),
            screen("b", 1920., 2560., 1440.),
        ];
        let was = saved(
            WindowBounds::Maximized(rect(0., 0., 1000., 700.)),
            Some("b"),
        );
        let (bounds, index) = place(&was, &screens, Some(0)).unwrap();
        assert_eq!(index, 1);
        assert_eq!(
            bounds,
            WindowBounds::Maximized(rect(1920. + 780., 0. + 0.5 * (1400. - 700.), 1000., 700.))
        );
    }

    #[test]
    fn a_window_barely_on_its_screen_is_centered() {
        let screens = [screen("a", 0., 1920., 1080.)];
        let centered = |x: f32, y: f32| {
            let was = saved(WindowBounds::Windowed(rect(x, y, 800., 600.)), Some("a"));
            place(&was, &screens, None).unwrap().0 != WindowBounds::Windowed(rect(x, y, 800., 600.))
        };
        assert!(centered(-799., 100.), "one pixel across");
        assert!(centered(100., -31.), "top edge above the screen");
        assert!(
            centered(100., 1030.),
            "less than 32 px left below the top edge"
        );
        assert!(centered(100., 1050.), "top edge below the screen");
        assert!(!centered(-700., 100.), "the top edge can still be grabbed");
    }

    #[test]
    fn a_failed_write_is_tried_again() {
        let home = std::env::temp_dir().join(format!("hoshi-window-{}", std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        let mut keeper = Keeper {
            home: home.clone(),
            written: None,
            saving: None,
        };
        let state = saved(WindowBounds::Maximized(rect(0., 0., 800., 600.)), None);
        keeper.save(state.clone());
        assert_eq!(keeper.written, None);
        std::fs::create_dir_all(&home).unwrap();
        keeper.save(state.clone());
        assert_eq!(keeper.written.as_ref(), Some(&state));
        assert_eq!(
            parse(&std::fs::read_to_string(file(&home)).unwrap()),
            Some(state.clone())
        );

        std::fs::remove_dir_all(&home).unwrap();
        keeper.save(saved(
            WindowBounds::Windowed(rect(0., 0., 800., 600.)),
            None,
        ));
        assert_eq!(keeper.written, None, "what was on disk may be gone");
        std::fs::create_dir_all(&home).unwrap();
        keeper.save(state.clone());
        assert_eq!(
            parse(&std::fs::read_to_string(file(&home)).unwrap()),
            Some(state)
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_missing_screen_falls_back_to_the_primary_and_big_windows_shrink() {
        let screens = [
            screen("a", 0., 1920., 1080.),
            screen("b", 1920., 1280., 800.),
        ];
        let was = saved(
            WindowBounds::Windowed(rect(5000., 0., 2560., 1400.)),
            Some("gone"),
        );
        assert_eq!(
            place(&was, &screens, Some(1)),
            Some((WindowBounds::Windowed(rect(1920., 0., 1280., 760.)), 1))
        );
        assert_eq!(place(&was, &screens, None).map(|(_, index)| index), Some(0));
        assert_eq!(place(&was, &[], None), None);
    }
}
