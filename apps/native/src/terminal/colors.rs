//! The terminal's colors: the theme's background, foreground and selection, and the ANSI
//! palette xterm.js ships, which the web terminal shows too.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors as Overrides;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::{Hsla, Rgba};
use hoshi_ui::theme::Colors;

const ANSI: [u32; 16] = [
    0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753,
    0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
];

const DIM: f32 = 0.66;

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub foreground: Hsla,
    pub background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub ansi: [Hsla; 16],
}

impl Palette {
    pub fn new(colors: &Colors) -> Self {
        Self {
            foreground: colors.foreground,
            background: colors.background,
            cursor: colors.foreground,
            selection: colors.terminal_selection,
            ansi: ANSI.map(|hex| Hsla::from(gpui_kit::rgb(hex))),
        }
    }

    /// The color at xterm's index: 16 named, a 6×6×6 cube, then 24 grays.
    pub fn indexed(&self, index: u8) -> Hsla {
        match index {
            0..16 => self.ansi[usize::from(index)],
            16..232 => {
                let cube = index - 16;
                let level = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
                rgb(Rgb {
                    r: level(cube / 36),
                    g: level(cube / 6 % 6),
                    b: level(cube % 6),
                })
            }
            232.. => {
                let gray = 8 + (index - 232) * 10;
                rgb(Rgb {
                    r: gray,
                    g: gray,
                    b: gray,
                })
            }
        }
    }

    fn named(&self, color: NamedColor) -> Hsla {
        let index = color as usize;
        match color {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => self.dim(self.foreground),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => self.dim(self.ansi[index - NamedColor::DimBlack as usize]),
            _ => self.ansi[index.min(15)],
        }
    }

    fn dim(&self, color: Hsla) -> Hsla {
        self.background.blend(color.opacity(DIM))
    }

    fn resolve(&self, color: Color, overrides: &Overrides) -> Hsla {
        let index = match color {
            Color::Spec(spec) => return rgb(spec),
            Color::Indexed(index) => usize::from(index),
            Color::Named(named) => named as usize,
        };
        if let Some(spec) = overrides[index] {
            return rgb(spec);
        }
        match color {
            Color::Indexed(index) => self.indexed(index),
            Color::Named(named) => self.named(named),
            Color::Spec(_) => unreachable!(),
        }
    }

    /// A cell's text and background colors, after bold-as-bright, dim, inverse and hidden.
    pub fn cell(&self, fg: Color, bg: Color, flags: Flags, overrides: &Overrides) -> (Hsla, Hsla) {
        let fg = if flags.contains(Flags::BOLD) {
            brighten(fg)
        } else {
            fg
        };
        let mut text = self.resolve(fg, overrides);
        let mut fill = self.resolve(bg, overrides);
        if flags.contains(Flags::DIM) {
            text = self.dim(text);
        }
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut text, &mut fill);
        }
        if flags.contains(Flags::HIDDEN) {
            text = fill;
        }
        (text, fill)
    }
}

fn brighten(color: Color) -> Color {
    match color {
        Color::Named(named) if (named as usize) < 8 => Color::Named(named.to_bright()),
        Color::Indexed(index) if index < 8 => Color::Indexed(index + 8),
        other => other,
    }
}

fn rgb(color: Rgb) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(color.r) / 255.,
        g: f32::from(color.g) / 255.,
        b: f32::from(color.b) / 255.,
        a: 1.,
    })
}

#[cfg(test)]
mod tests {
    use super::{Palette, rgb};
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::color::Colors as Overrides;
    use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
    use gpui_kit::component::theme::ThemeMode;

    fn palette() -> Palette {
        Palette::new(&hoshi_ui::theme::colors(ThemeMode::Dark))
    }

    #[test]
    fn defaults_take_the_theme_colors() {
        let palette = palette();
        let (text, fill) = palette.cell(
            Color::Named(NamedColor::Foreground),
            Color::Named(NamedColor::Background),
            Flags::empty(),
            &Overrides::default(),
        );
        assert_eq!(text, palette.foreground);
        assert_eq!(fill, palette.background);
    }

    #[test]
    fn indexed_colors_follow_xterm() {
        let palette = palette();
        assert_eq!(palette.indexed(1), palette.ansi[1]);
        assert_eq!(palette.indexed(16), rgb(Rgb { r: 0, g: 0, b: 0 }));
        assert_eq!(palette.indexed(196), rgb(Rgb { r: 255, g: 0, b: 0 }));
        assert_eq!(
            palette.indexed(231),
            rgb(Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(palette.indexed(232), rgb(Rgb { r: 8, g: 8, b: 8 }));
        assert_eq!(
            palette.indexed(255),
            rgb(Rgb {
                r: 238,
                g: 238,
                b: 238
            })
        );
    }

    #[test]
    fn bold_brightens_and_inverse_swaps() {
        let palette = palette();
        let none = Overrides::default();
        let red = Color::Named(NamedColor::Red);
        let background = Color::Named(NamedColor::Background);
        let (bold, _) = palette.cell(red, background, Flags::BOLD, &none);
        assert_eq!(bold, palette.ansi[9]);
        let (text, fill) = palette.cell(red, background, Flags::INVERSE, &none);
        assert_eq!((text, fill), (palette.background, palette.ansi[1]));
        let (hidden, fill) = palette.cell(red, background, Flags::HIDDEN, &none);
        assert_eq!(hidden, fill);
    }

    #[test]
    fn a_color_the_program_set_wins() {
        let palette = palette();
        let mut overrides = Overrides::default();
        overrides[NamedColor::Red as usize] = Some(Rgb { r: 1, g: 2, b: 3 });
        let (text, _) = palette.cell(
            Color::Named(NamedColor::Red),
            Color::Named(NamedColor::Background),
            Flags::empty(),
            &overrides,
        );
        assert_eq!(text, rgb(Rgb { r: 1, g: 2, b: 3 }));
        let spec = Color::Spec(Rgb { r: 9, g: 9, b: 9 });
        let (text, _) = palette.cell(spec, spec, Flags::empty(), &overrides);
        assert_eq!(text, rgb(Rgb { r: 9, g: 9, b: 9 }));
    }
}
