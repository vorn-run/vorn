//! The theme a UI thread draws with, live: every [`Color`] the widgets and
//! the app name is a token looked up here when a frame is built, so
//! installing another theme recolours every window on its next frame.
//!
//! Per thread rather than global: the windows of one app share their UI
//! thread, and tests drawing different themes side by side do not meet.

use std::cell::RefCell;
use std::rc::Rc;

use vorn_theme::{Materials, Rgba8, Theme, Token};

use crate::scene::Rgba;

/// A colour as the UI names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    /// A theme token.
    Token(Token),
    /// A token at `alpha` times its own: hairlines, washes, scrims.
    Faded(Token, f32),
    /// A colour that is content rather than chrome, as `0xRRGGBB`: one a
    /// person picked, an agent's or a file type's mark.
    Content(u32),
}

impl Color {
    /// The colour the current theme gives it.
    pub fn rgba(self) -> Rgba {
        match self {
            Color::Token(t) => get(t),
            Color::Faded(t, a) => get(t).alpha(a),
            Color::Content(c) => Rgba::hex(c),
        }
    }

    /// [`Color::rgba`] with its alpha multiplied by `a`.
    pub fn alpha(self, a: f32) -> Rgba {
        self.rgba().alpha(a)
    }

    /// [`Color::rgba`] mixed towards `to`.
    pub fn mix(self, to: impl Into<Rgba>, t: f32) -> Rgba {
        self.rgba().mix(to.into(), t)
    }

    /// The same token at `alpha` times its own, for constants.
    pub const fn faded(self, alpha: f32) -> Color {
        match self {
            Color::Token(t) => Color::Faded(t, alpha),
            Color::Faded(t, a) => Color::Faded(t, a * alpha),
            other => other,
        }
    }

    /// `0xRRGGBB` under the current theme, for code that keeps colours as numbers.
    pub fn hex(self) -> u32 {
        self.rgba().to_hex()
    }
}

impl From<Color> for Rgba {
    fn from(c: Color) -> Rgba {
        c.rgba()
    }
}

impl From<Token> for Color {
    fn from(t: Token) -> Color {
        Color::Token(t)
    }
}

/// A content colour as `0xRRGGBB`.
impl From<u32> for Color {
    fn from(c: u32) -> Color {
        Color::Content(c)
    }
}

impl From<Rgba8> for Rgba {
    fn from(c: Rgba8) -> Rgba {
        Rgba::rgba_const(c.r, c.g, c.b, f32::from(c.a) / 255.0)
    }
}

/// The terminal's colours, ready to paint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Terminal {
    pub foreground: Rgba,
    pub background: Rgba,
    pub cursor: Rgba,
    pub selection: Rgba,
    pub ansi: [Rgba; 16],
}

/// One installed theme, resolved for painting.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    /// The theme it came from; the settings and the terminal read it.
    pub theme: Theme,
    pub ui: [Rgba; Token::COUNT],
    pub terminal: Terminal,
    pub materials: Materials,
}

impl Palette {
    pub fn new(theme: Theme, materials: Materials) -> Palette {
        let ui = theme.tokens().map(Rgba::from);
        let t = &theme.terminal;
        let terminal = Terminal {
            foreground: t.foreground.into(),
            background: t.background.into(),
            cursor: t.cursor.into(),
            selection: t.selection.into(),
            ansi: t.ansi.map(Rgba::from),
        };
        Palette {
            theme,
            ui,
            terminal,
            materials,
        }
    }

    /// Whether the window is dark.
    pub fn dark(&self) -> bool {
        self.theme.appearance == vorn_theme::Appearance::Dark
    }
}

impl Default for Palette {
    /// Vorn Dark, solid: today's look.
    fn default() -> Palette {
        Palette::new(vorn_theme::vorn_dark().clone(), Materials::Solid)
    }
}

thread_local! {
    static CURRENT: RefCell<(Rc<Palette>, u64)> = RefCell::new((Rc::new(Palette::default()), 0));
}

/// Draws this thread's windows with `palette` from their next frame;
/// `false` when it is the one already installed.
pub fn install(palette: Palette) -> bool {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        if *c.0 == palette {
            return false;
        }
        c.0 = Rc::new(palette);
        c.1 += 1;
        true
    })
}

/// The palette this thread draws with.
pub fn current() -> Rc<Palette> {
    CURRENT.with(|c| c.borrow().0.clone())
}

/// Counts installs, so a cache of drawn colours knows when it is stale.
pub fn generation() -> u64 {
    CURRENT.with(|c| c.borrow().1)
}

/// The colour of `token` in the current theme.
pub fn get(token: Token) -> Rgba {
    CURRENT.with(|c| c.borrow().0.ui[token as usize])
}

/// The brightest ink at `a`: white on a dark theme, black on a light one.
/// Washes, hairlines and highlights are drawn with it.
pub fn bright(a: f32) -> Rgba {
    get(Token::InkBright).alpha(a)
}

/// The shadow colour at `a`: scrims and drop shadows.
pub fn shadow(a: f32) -> Rgba {
    get(Token::Shadow).alpha(a)
}

/// The terminal's colours in the current theme.
pub fn terminal() -> Terminal {
    CURRENT.with(|c| c.borrow().0.terminal)
}

/// Whether the current theme is dark.
pub fn dark() -> bool {
    CURRENT.with(|c| c.borrow().0.dark())
}

/// The current materials.
pub fn materials() -> Materials {
    CURRENT.with(|c| c.borrow().0.materials)
}

/// Whether the platform asks for less transparency:
/// `VORNUI_REDUCE_TRANSPARENCY=1`, macOS's Reduce transparency, or Windows'
/// Transparency effects switched off.
pub fn system_reduced_transparency() -> bool {
    if let Some(v) = std::env::var_os("VORNUI_REDUCE_TRANSPARENCY") {
        return v != "0" && !v.is_empty();
    }
    platform_reduced_transparency()
}

#[cfg(target_os = "macos")]
fn platform_reduced_transparency() -> bool {
    crate::macos::reduce_transparency()
}

/// Windows' Transparency effects switch, read from the registry.
#[cfg(windows)]
fn platform_reduced_transparency() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("EnableTransparency");
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the strings are NUL-terminated and `value` holds the DWORD `size` says.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &raw mut size,
        )
    };
    transparency_switch_reduces((status == 0).then_some(value))
}

/// `EnableTransparency` as read: 0 turns transparency effects off; a
/// missing value leaves them on, as Windows does.
#[cfg_attr(not(windows), allow(dead_code))]
fn transparency_switch_reduces(enable_transparency: Option<u32>) -> bool {
    enable_transparency == Some(0)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_reduced_transparency() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_transparency_off_reads_as_reduced() {
        assert!(transparency_switch_reduces(Some(0)));
        assert!(!transparency_switch_reduces(Some(1)));
        assert!(!transparency_switch_reduces(None));
    }

    #[cfg(windows)]
    #[test]
    fn the_registry_read_answers_without_failing() {
        let _ = platform_reduced_transparency();
    }

    #[test]
    fn installing_recolours_tokens_and_counts() {
        let before = generation();
        let base = Color::Token(Token::SurfaceBase);
        assert_eq!(base.hex(), 0x0d0d0f);
        let light = vorn_theme::find_bundled(vorn_theme::VORN_LIGHT)
            .unwrap()
            .clone();
        assert!(install(Palette::new(light.clone(), Materials::Solid)));
        assert!(!install(Palette::new(light, Materials::Solid)));
        assert_eq!(generation(), before + 1);
        assert_eq!(base.hex(), 0xf7f7f5);
        assert_eq!(bright(1.0), Rgba::BLACK);
        assert!(!dark());
        assert_eq!(Color::Content(0x123456).hex(), 0x123456);
        install(Palette::default());
        assert_eq!(bright(0.5), Rgba([1.0, 1.0, 1.0, 0.5]));
    }

    #[test]
    fn faded_multiplies_alpha() {
        let line = Color::Token(Token::InkBright).faded(0.5).faded(0.5);
        assert!((line.rgba().0[3] - 0.25).abs() < 1e-6);
    }
}
