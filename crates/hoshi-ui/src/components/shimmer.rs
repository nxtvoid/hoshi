use super::looping::{LoopExt as _, looping};
use gpui_kit::component::{ActiveTheme as _, Colorize as _};
use gpui_kit::*;
use std::time::Duration;

const SWEEP: Duration = Duration::from_secs(2);
/// Half the band's width, as a share of the text's.
const SPREAD: f32 = 0.3;
/// Nested bands, each narrower and painted over the last, so the light fades toward the edges.
const LAYERS: usize = 12;

/// Text a band of light sweeps across, drawn on the loops' clock (see [`super::LOOP_FPS`]) so a
/// running label doesn't redraw the window on every frame of the display.
#[derive(IntoElement)]
pub struct Shimmer {
    text: SharedString,
    style: StyleRefinement,
}

impl Shimmer {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: StyleRefinement::default(),
        }
    }
}

impl Styled for Shimmer {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Shimmer {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme.semantic_tokens();
        let glyphs = Glyphs {
            text: StyledText::new(self.text),
            background: tokens.colors.background,
            foreground: tokens.colors.foreground,
            dark: theme.is_dark(),
            phase: 0.,
        };
        let mut container = div().min_w_0();
        container.style().refine(&self.style);
        container.child(glyphs.with_loop(looping(SWEEP), |mut glyphs, phase| {
            glyphs.phase = phase;
            glyphs
        }))
    }
}

/// The text as `StyledText` lays it out, with the band painted over its glyphs.
struct Glyphs {
    text: StyledText,
    background: Hsla,
    foreground: Hsla,
    dark: bool,
    phase: f32,
}

impl IntoElement for Glyphs {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Glyphs {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .prepaint(id, inspector_id, bounds, layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(id, inspector_id, bounds, layout, prepaint, window, cx);
        self.paint_band(bounds, window, cx);
    }
}

impl Glyphs {
    fn paint_band(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let masks: Vec<ContentMask<Pixels>> = (0..LAYERS)
            .filter_map(|layer| band(bounds, self.phase, layer))
            .map(|bounds| ContentMask { bounds })
            .collect();
        if masks.is_empty() {
            return;
        }
        let color = self.highlight(window.text_style().color);
        let layout = self.text.layout();
        let line_height = layout.line_height();
        let align = window.text_style().text_align;
        let mut line_top = bounds.top();
        window.paint_layer(bounds, |window| {
            for wrapped in layout.line_layouts() {
                let line = &wrapped.unwrapped_layout;
                let baseline = (line_height - line.ascent - line.descent) / 2. + line.ascent;
                let mut wraps = wrapped.wrap_boundaries.iter().peekable();
                let mut row_top = line_top;
                let mut row_start = px(0.);
                let mut row_left = aligned_left(bounds, align, row_end(line, wraps.peek()));
                for (run_ix, run) in line.runs.iter().enumerate() {
                    let glyph_size = cx
                        .text_system()
                        .bounding_box(run.font_id, line.font_size)
                        .size;
                    for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                        if wraps
                            .peek()
                            .is_some_and(|wrap| wrap.run_ix == run_ix && wrap.glyph_ix == glyph_ix)
                        {
                            wraps.next();
                            row_top += line_height;
                            row_start = glyph.position.x;
                            row_left = aligned_left(
                                bounds,
                                align,
                                row_end(line, wraps.peek()) - row_start,
                            );
                        }
                        if glyph.is_emoji {
                            continue;
                        }
                        let origin = point(row_left + glyph.position.x - row_start, row_top);
                        let glyph_bounds = Bounds::new(origin, glyph_size);
                        let at = origin + point(px(0.), baseline + glyph.position.y);
                        for mask in &masks {
                            if glyph_bounds.intersects(&mask.bounds) {
                                window.with_content_mask(Some(*mask), |window| {
                                    _ = window.paint_glyph(
                                        at,
                                        run.font_id,
                                        glyph.id,
                                        line.font_size,
                                        color,
                                    );
                                });
                            }
                        }
                    }
                }
                line_top += wrapped.size(line_height).height;
            }
        });
    }

    /// The text's color moved a fifth of the way toward the theme's brightest end, see-through
    /// enough that the layers add up to the band's peak.
    fn highlight(&self, text: Hsla) -> Hsla {
        let (toward, away) = if self.dark {
            (self.foreground, self.background)
        } else {
            (self.background, self.foreground)
        };
        let toward = if (toward.l - text.l).abs() < 0.1 {
            away
        } else {
            toward
        };
        let peak: f32 = if self.dark { 0.6 } else { 0.75 };
        let layer = 1. - (1. - peak).powf(1. / LAYERS as f32);
        text.mix_oklab(toward, 0.2).opacity(layer)
    }
}

/// Where a row's glyphs end, measured from the start of its line.
fn row_end(line: &LineLayout, next_wrap: Option<&&WrapBoundary>) -> Pixels {
    next_wrap.map_or(line.width, |wrap| {
        line.runs[wrap.run_ix].glyphs[wrap.glyph_ix].position.x
    })
}

/// Where a row `width` wide starts, as `StyledText` places it.
fn aligned_left(bounds: Bounds<Pixels>, align: TextAlign, width: Pixels) -> Pixels {
    match align {
        TextAlign::Left => bounds.left(),
        TextAlign::Center => bounds.left() + (bounds.size.width - width) / 2.,
        TextAlign::Right => bounds.right() - width,
    }
}

/// Where `layer` of the band is at `phase`: it starts left of the text and leaves past its end.
fn band(bounds: Bounds<Pixels>, phase: f32, layer: usize) -> Option<Bounds<Pixels>> {
    let width = bounds.size.width.as_f32();
    if width <= 0. || bounds.size.height <= px(0.) {
        return None;
    }
    let half = width * SPREAD;
    let margin = SPREAD + 0.05;
    let center = (phase * (1. + 2. * margin) - margin) * width;
    let radius = half * (1. - layer as f32 / LAYERS as f32);
    let left = (center - radius).max(0.);
    let right = (center + radius).min(width);
    (right > left).then(|| {
        Bounds::new(
            point(bounds.left() + px(left), bounds.top()),
            size(px(right - left), bounds.size.height),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{LAYERS, aligned_left, band};
    use gpui_kit::{Bounds, Pixels, TextAlign, point, px, size};

    fn bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.), px(0.)), size(px(100.), px(20.)))
    }

    #[test]
    fn the_band_starts_and_ends_outside_the_text() {
        assert_eq!(band(bounds(), 0., 0), None);
        assert_eq!(band(bounds(), 1., 0), None);
    }

    #[test]
    fn halfway_the_band_is_centered_and_its_layers_narrow() {
        let outer = band(bounds(), 0.5, 0).unwrap();
        let inner = band(bounds(), 0.5, LAYERS - 1).unwrap();
        assert_eq!(outer.center().x, px(60.));
        assert_eq!(outer.size.width, px(60.));
        assert!(inner.size.width < outer.size.width);
        assert_eq!(inner.center().x, px(60.));
    }

    #[test]
    fn a_row_starts_where_its_alignment_puts_it() {
        assert_eq!(aligned_left(bounds(), TextAlign::Left, px(40.)), px(10.));
        assert_eq!(aligned_left(bounds(), TextAlign::Center, px(40.)), px(40.));
        assert_eq!(aligned_left(bounds(), TextAlign::Right, px(40.)), px(70.));
    }
}
