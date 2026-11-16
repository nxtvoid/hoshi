use gpui_kit::{Bounds, Keystroke, Pixels, Point, Size, point, px, size};
use hoshi_core::BrowserInput;

/// The sizes the panel offers besides filling it, in CSS pixels.
pub const PAGE_SIZES: [(u32, u32); 3] = [(1280, 800), (768, 1024), (390, 844)];

/// The size a page of its own size shows at inside the panel: whole, and never larger than
/// it is.
pub fn fit_inside(panel: Size<Pixels>, page: (u32, u32)) -> Size<Pixels> {
    let (width, height) = (page.0 as f32, page.1 as f32);
    if width <= 0. || height <= 0. {
        return Size::default();
    }
    let scale = 1f32
        .min(f32::from(panel.width) / width)
        .min(f32::from(panel.height) / height)
        .max(0.);
    size(px((width * scale).floor()), px((height * scale).floor()))
}

/// Where the frame shows inside the viewport: across its whole width from the top when the
/// page fills the panel, else whole and centered.
pub fn page_bounds(viewport: Bounds<Pixels>, frame: (u32, u32), fill: bool) -> Bounds<Pixels> {
    if fill {
        let height = if frame.0 == 0 {
            px(0.)
        } else {
            viewport.size.width * (frame.1 as f32 / frame.0 as f32)
        };
        return Bounds::new(viewport.origin, size(viewport.size.width, height));
    }
    let shown = fit_inside(viewport.size, frame);
    let origin = point(
        viewport.origin.x + (viewport.size.width - shown.width) / 2.,
        viewport.origin.y + (viewport.size.height - shown.height) / 2.,
    );
    Bounds::new(origin, shown)
}

/// A point of the window over the shown frame, in the page's own pixels.
pub fn page_point(
    shown: Bounds<Pixels>,
    frame_width: u32,
    position: Point<Pixels>,
) -> Option<(f64, f64)> {
    if shown.size.width <= px(0.) || !shown.contains(&position) {
        return None;
    }
    let scale = frame_width as f64 / f64::from(f32::from(shown.size.width));
    Some((
        f64::from(f32::from(position.x - shown.origin.x)) * scale,
        f64::from(f32::from(position.y - shown.origin.y)) * scale,
    ))
}

/// What a key pressed on the page sends: named keys as the page knows them, printable ones
/// as text. Shortcuts stay with the app.
pub fn key_input(keystroke: &Keystroke) -> Option<BrowserInput> {
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform {
        return None;
    }
    let named = match keystroke.key.as_str() {
        "enter" => "Enter",
        "tab" => "Tab",
        "escape" => "Escape",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "left" => "ArrowLeft",
        "up" => "ArrowUp",
        "right" => "ArrowRight",
        "down" => "ArrowDown",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        _ => {
            let text = keystroke.key_char.as_deref()?;
            if text.is_empty() || text.chars().any(char::is_control) {
                return None;
            }
            return Some(BrowserInput::Text {
                text: text.to_owned(),
            });
        }
    };
    Some(BrowserInput::Key {
        key: named.to_owned(),
    })
}

/// A wheel turn as the page scrolls it: GPUI's deltas move the content, the page's move the
/// view.
pub fn page_scroll(delta: Point<Pixels>) -> (f64, f64) {
    (
        -f64::from(f32::from(delta.x)),
        -f64::from(f32::from(delta.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::{fit_inside, key_input, page_bounds, page_point, page_scroll};
    use gpui_kit::{Bounds, Keystroke, point, px, size};
    use hoshi_core::BrowserInput;

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui_kit::Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn a_page_shows_whole_and_never_larger_than_it_is() {
        let panel = size(px(400.), px(600.));
        assert_eq!(fit_inside(panel, (1280, 800)), size(px(400.), px(250.)));
        assert_eq!(fit_inside(panel, (390, 844)), size(px(277.), px(600.)));
        assert_eq!(fit_inside(panel, (200, 100)), size(px(200.), px(100.)));
        assert_eq!(fit_inside(panel, (0, 100)), size(px(0.), px(0.)));
    }

    #[test]
    fn a_filling_page_starts_at_the_top_and_a_sized_one_is_centered() {
        let viewport = bounds(10., 20., 400., 600.);
        assert_eq!(
            page_bounds(viewport, (800, 1000), true),
            bounds(10., 20., 400., 500.)
        );
        assert_eq!(
            page_bounds(viewport, (1280, 800), false),
            bounds(10., 195., 400., 250.)
        );
    }

    #[test]
    fn clicks_map_to_the_pages_own_pixels_and_miss_outside_it() {
        let shown = bounds(10., 195., 400., 250.);
        assert_eq!(
            page_point(shown, 1280, point(px(110.), px(245.))),
            Some((320., 160.))
        );
        assert_eq!(page_point(shown, 1280, point(px(110.), px(100.))), None);
        assert_eq!(
            page_point(bounds(0., 0., 0., 0.), 1280, point(px(0.), px(0.))),
            None
        );
    }

    #[test]
    fn named_keys_go_by_name_text_as_text_and_shortcuts_stay() {
        let key = |text: &'static str| key_input(&Keystroke::parse(text).unwrap());
        assert_eq!(
            key("enter"),
            Some(BrowserInput::Key {
                key: "Enter".into()
            })
        );
        assert_eq!(
            key("left"),
            Some(BrowserInput::Key {
                key: "ArrowLeft".into()
            })
        );
        let mut typed = Keystroke::parse("shift-a").unwrap();
        typed.key_char = Some("A".into());
        assert_eq!(
            key_input(&typed),
            Some(BrowserInput::Text { text: "A".into() })
        );
        assert_eq!(key("ctrl-a"), None);
        assert_eq!(key("f5"), None);
    }

    #[test]
    fn a_wheel_turn_down_scrolls_the_page_down() {
        assert_eq!(page_scroll(point(px(0.), px(-40.))), (-0., 40.));
    }
}
