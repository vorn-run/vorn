//! The tokens a theme does not set, derived from the ones it does and its
//! terminal palette: surfaces step from the background towards the ink,
//! grays mix the two, and each hue family comes from its ANSI colours.

use crate::{Appearance, Rgba8, Terminal, Token};

/// Every token: the given ones as written, the rest derived.
pub(crate) fn tokens(
    appearance: Appearance,
    terminal: &Terminal,
    given: &[Option<Rgba8>; Token::COUNT],
) -> [Rgba8; Token::COUNT] {
    let mut d = Deriver {
        dark: appearance == Appearance::Dark,
        terminal,
        given,
        done: [None; Token::COUNT],
    };
    let mut out = [Rgba8::BLACK; Token::COUNT];
    for &t in Token::ALL {
        out[t as usize] = d.get(t);
    }
    out
}

/// A hue family's normal and bright ANSI colours.
#[derive(Clone, Copy)]
enum Hue {
    Ansi(usize),
    /// `t` of the way from one family to another.
    Mix(usize, usize, f32),
}

struct Deriver<'a> {
    dark: bool,
    terminal: &'a Terminal,
    given: &'a [Option<Rgba8>; Token::COUNT],
    done: [Option<Rgba8>; Token::COUNT],
}

impl Deriver<'_> {
    fn get(&mut self, t: Token) -> Rgba8 {
        if let Some(c) = self.done[t as usize] {
            return c;
        }
        let c = match self.given[t as usize] {
            Some(c) => c,
            None => self.recipe(t),
        };
        self.done[t as usize] = Some(c);
        c
    }

    fn ink(&mut self, a: f32) -> Rgba8 {
        self.get(Token::Ink).with_alpha(a)
    }

    fn gray(&mut self, w: f32) -> Rgba8 {
        let (base, ink) = (self.get(Token::SurfaceBase), self.get(Token::Ink));
        base.mix(ink, w)
    }

    /// The family's (normal, bright) pair.
    fn hue(&self, hue: Hue) -> (Rgba8, Rgba8) {
        let ansi = &self.terminal.ansi;
        match hue {
            Hue::Ansi(i) => (ansi[i], ansi[i + 8]),
            Hue::Mix(a, b, t) => (ansi[a].mix(ansi[b], t), ansi[a + 8].mix(ansi[b + 8], t)),
        }
    }

    /// A Tailwind step of a family: 200 and 300 lean to the ink, 400 is the
    /// bright colour, 500 the normal one, 600 leans to the window.
    fn step(&mut self, hue: Hue, step: u16) -> Rgba8 {
        let (normal, bright) = self.hue(hue);
        match step {
            200 => bright.mix(self.get(Token::Ink), 0.5),
            300 => bright.mix(self.get(Token::Ink), 0.25),
            400 => bright,
            500 => normal,
            _ => normal.mix(self.get(Token::SurfaceBase), 0.2),
        }
    }

    fn recipe(&mut self, t: Token) -> Rgba8 {
        use Token::*;
        const RED: Hue = Hue::Ansi(1);
        const GREEN: Hue = Hue::Ansi(2);
        const YELLOW: Hue = Hue::Ansi(3);
        const BLUE: Hue = Hue::Ansi(4);
        const CYAN: Hue = Hue::Ansi(6);
        const ORANGE: Hue = Hue::Mix(1, 3, 0.45);
        const AMBER: Hue = Hue::Mix(3, 1, 0.2);
        const EMERALD: Hue = Hue::Mix(2, 6, 0.4);
        const SKY: Hue = Hue::Mix(6, 4, 0.35);
        const VIOLET: Hue = Hue::Mix(4, 5, 0.5);
        const PURPLE: Hue = Hue::Mix(5, 4, 0.25);
        const PINK: Hue = Hue::Mix(5, 1, 0.3);
        let dark = self.dark;
        let toward_ink = |d: &mut Self, w: f32| {
            let (base, ink) = (d.get(SurfaceBase), d.get(Ink));
            base.mix(ink, w)
        };
        match t {
            SurfaceBase => self.terminal.background,
            SurfacePanel => toward_ink(self, if dark { 0.012 } else { 0.025 }),
            SurfaceSunken => toward_ink(self, if dark { 0.028 } else { 0.045 }),
            SurfaceOverlay if dark => toward_ink(self, 0.06),
            SurfaceOverlay => self.get(SurfaceBase).mix(Rgba8::WHITE, 0.6),
            Ink => self.terminal.foreground,
            InkBright => {
                let extreme = if dark { Rgba8::WHITE } else { Rgba8::BLACK };
                self.get(Ink).mix(extreme, 0.5)
            }
            InkSecondary => self.ink(0.55),
            InkFaint => self.ink(0.35),
            InkGhost => self.ink(0.18),
            LineFaint => self.ink(0.04),
            LineSoft => self.ink(0.06),
            LineBase => self.ink(0.08),
            LineStrong => self.ink(0.10),
            LineHover => self.ink(0.12),
            Accent => self.terminal.ansi[3],
            AccentStrong => {
                let base = self.get(SurfaceBase);
                self.get(Accent).mix(base, 0.12)
            }
            Danger | DiffRemove => self.terminal.ansi[1],
            DiffAdd => self.terminal.ansi[2],
            StatusSlate => self.gray(0.5),
            StatusBlue => {
                let slate = self.get(StatusSlate);
                self.terminal.ansi[4].mix(slate, 0.35)
            }
            StatusSage => {
                let slate = self.get(StatusSlate);
                self.terminal.ansi[2].mix(slate, 0.45)
            }
            Selection => self.step(BLUE, 500).with_alpha(0.4),
            FocusRing => self.ink(0.45),
            ScrollThumb => self.ink(0.08),
            ScrollThumbHover => self.ink(0.2),
            Shadow => Rgba8::BLACK,
            Gray100 => self.gray(0.94),
            Gray200 => self.gray(0.88),
            Gray300 => self.gray(0.8),
            Gray400 => self.gray(0.6),
            Gray500 => self.gray(0.44),
            Gray600 => self.gray(0.32),
            Gray700 => self.gray(0.23),
            Gray800 => self.gray(0.14),
            Gray900 => self.gray(0.07),
            Red200 => self.step(RED, 200),
            Red300 => self.step(RED, 300),
            Red400 => self.step(RED, 400),
            Red500 => self.step(RED, 500),
            Red600 => self.step(RED, 600),
            Orange400 => self.step(ORANGE, 400),
            Orange500 => self.step(ORANGE, 500),
            Amber300 => self.step(AMBER, 300),
            Amber400 => self.step(AMBER, 400),
            Amber500 => self.step(AMBER, 500),
            Yellow400 => self.step(YELLOW, 400),
            Yellow500 => self.step(YELLOW, 500),
            Green300 => self.step(GREEN, 300),
            Green400 => self.step(GREEN, 400),
            Green500 => self.step(GREEN, 500),
            Green600 => self.step(GREEN, 600),
            Emerald400 => self.step(EMERALD, 400),
            Cyan400 => self.step(CYAN, 400),
            Sky400 => self.step(SKY, 400),
            Blue300 => self.step(BLUE, 300),
            Blue400 => self.step(BLUE, 400),
            Blue500 => self.step(BLUE, 500),
            Blue600 => self.step(BLUE, 600),
            Violet400 => self.step(VIOLET, 400),
            Violet500 => self.step(VIOLET, 500),
            Purple400 => self.step(PURPLE, 400),
            Pink400 => self.step(PINK, 400),
            Pink500 => self.step(PINK, 500),
            SyntaxText | SyntaxVariable => self.get(Ink),
            SyntaxComment | SyntaxPunct => self.get(Gray500),
            SyntaxString => self.terminal.ansi[2],
            SyntaxNumber | SyntaxProperty => self.terminal.ansi[6],
            SyntaxKeyword => self.terminal.ansi[5],
            SyntaxStorage => self.terminal.ansi[1],
            SyntaxConstant | SyntaxList => self.get(Orange400),
            SyntaxFunction => self.terminal.ansi[4],
            SyntaxType => self.terminal.ansi[3],
        }
    }
}
