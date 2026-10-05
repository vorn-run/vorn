//! A session's style and link tables (Terminal State Protocol §6): ids
//! handed out in order within one `(state_gen, table_gen)`, never reused or
//! redefined, so a client that holds ids `0..mark` can always draw a row.
//!
//! Id 0 of each table is the default: the default style, and "no link". A
//! table that passes its limit is compacted by starting it again under the
//! next `table_gen`; the grid then re-encodes every row against the fresh
//! table, which renumbers exactly the definitions still on the viewport.
//! History rows are encoded when they are fetched, so they need no
//! renumbering.

use std::collections::HashMap;

use libghostty_vt::style::{Style, StyleColor, Underline as GhosttyUnderline};
use vorn_term_proto::screen::{attrs, Color, LinkDef, StyleDef, Underline};

/// Past this many styles the table is compacted (TP §6).
pub const MAX_STYLES: usize = 4096;
/// Past this many links the table is compacted.
pub const MAX_LINKS: usize = 1024;

/// A style as a hash key: colours packed as on the wire, then the
/// attributes and the underline kind.
type StyleKey = (u32, u32, u32, u16, u8);

/// A session's style and link definitions, in id order.
#[derive(Debug)]
pub struct Tables {
    gen: u32,
    styles: Vec<StyleDef>,
    by_style: HashMap<StyleKey, u32>,
    links: Vec<LinkDef>,
    by_link: HashMap<String, u32>,
}

impl Default for Tables {
    fn default() -> Self {
        let mut t = Tables {
            gen: 0,
            styles: Vec::new(),
            by_style: HashMap::new(),
            links: Vec::new(),
            by_link: HashMap::new(),
        };
        t.seed();
        t
    }
}

impl Tables {
    /// The table generation, raised by each compaction.
    pub fn gen(&self) -> u32 {
        self.gen
    }

    pub fn styles(&self) -> &[StyleDef] {
        &self.styles
    }

    pub fn links(&self) -> &[LinkDef] {
        &self.links
    }

    /// Whether a table passed its limit and should be compacted before the
    /// next frame is encoded.
    pub fn over_limit(&self) -> bool {
        self.styles.len() > MAX_STYLES || self.links.len() > MAX_LINKS
    }

    /// Starts both tables again under the next generation.
    pub fn compact(&mut self) {
        self.gen = self.gen.wrapping_add(1);
        self.styles.clear();
        self.by_style.clear();
        self.links.clear();
        self.by_link.clear();
        self.seed();
    }

    fn seed(&mut self) {
        let def = StyleDef::default();
        self.by_style.insert(key(&def), 0);
        self.styles.push(def);
        self.links.push(LinkDef::default());
    }

    /// The id of `style`, with the background of a cell that holds only a
    /// background colour laid over it, minting one if it is new.
    pub fn style(&mut self, style: &Style, bg: Option<Color>) -> u32 {
        let mut def = style_def(style);
        if let Some(bg) = bg {
            def.bg = bg;
        }
        self.style_def(def)
    }

    /// The id of a style already in wire form.
    pub fn style_def(&mut self, mut def: StyleDef) -> u32 {
        let k = key(&def);
        if let Some(&id) = self.by_style.get(&k) {
            return id;
        }
        let id = self.styles.len() as u32;
        def.id = id;
        self.styles.push(def);
        self.by_style.insert(k, id);
        id
    }

    /// The id of a hyperlink by its URI. Ghostty does not expose OSC 8's
    /// `id=` parameter, so two links with one URI share an id.
    pub fn link(&mut self, uri: &str) -> u32 {
        if let Some(&id) = self.by_link.get(uri) {
            return id;
        }
        let id = self.links.len() as u32;
        self.links.push(LinkDef {
            id,
            uri: uri.to_owned(),
            osc8_id: None,
        });
        self.by_link.insert(uri.to_owned(), id);
        id
    }
}

fn key(s: &StyleDef) -> StyleKey {
    (
        color_key(s.fg),
        color_key(s.bg),
        color_key(s.underline_color),
        s.attrs,
        s.underline as u8,
    )
}

fn color_key(c: Color) -> u32 {
    match c {
        Color::Default => 0,
        Color::Palette(i) => 1 << 24 | u32::from(i),
        Color::Rgb(r, g, b) => 2 << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
    }
}

/// Ghostty's colour, unresolved: a palette change resends no rows.
pub fn color(c: StyleColor) -> Color {
    match c {
        StyleColor::None => Color::Default,
        StyleColor::Palette(i) => Color::Palette(i.0),
        StyleColor::Rgb(c) => Color::Rgb(c.r, c.g, c.b),
    }
}

/// Ghostty's style in wire form, id 0 until the table gives it one.
pub fn style_def(s: &Style) -> StyleDef {
    let mut a = 0;
    for (on, bit) in [
        (s.bold, attrs::BOLD),
        (s.italic, attrs::ITALIC),
        (s.faint, attrs::FAINT),
        (s.blink, attrs::BLINK),
        (s.inverse, attrs::INVERSE),
        (s.invisible, attrs::INVISIBLE),
        (s.strikethrough, attrs::STRIKE),
        (s.overline, attrs::OVERLINE),
    ] {
        if on {
            a |= bit;
        }
    }
    StyleDef {
        id: 0,
        fg: color(s.fg_color),
        bg: color(s.bg_color),
        underline_color: color(s.underline_color),
        attrs: a,
        underline: match s.underline {
            GhosttyUnderline::None => Underline::None,
            GhosttyUnderline::Single => Underline::Single,
            GhosttyUnderline::Double => Underline::Double,
            GhosttyUnderline::Curly => Underline::Curly,
            GhosttyUnderline::Dotted => Underline::Dotted,
            GhosttyUnderline::Dashed => Underline::Dashed,
            _ => Underline::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::style::{PaletteIndex, RgbColor};

    #[test]
    fn ids_are_handed_out_once_in_order() {
        let mut t = Tables::default();
        assert_eq!(t.style(&Style::default(), None), 0);
        let red = Style {
            fg_color: StyleColor::Palette(PaletteIndex(1)),
            ..Style::default()
        };
        assert_eq!(t.style(&red, None), 1);
        assert_eq!(t.style(&red, None), 1);
        // A background-only cell over the default style is a style of its own.
        assert_eq!(t.style(&Style::default(), Some(Color::Rgb(1, 2, 3))), 2);
        assert_eq!(t.styles()[2].bg, Color::Rgb(1, 2, 3));
        assert_eq!(t.link("https://a"), 1);
        assert_eq!(t.link("https://b"), 2);
        assert_eq!(t.link("https://a"), 1);
        assert!(t.styles().iter().enumerate().all(|(i, s)| s.id == i as u32));
    }

    #[test]
    fn compaction_starts_a_new_generation_from_the_defaults() {
        let mut t = Tables::default();
        for i in 0..=MAX_STYLES as u32 {
            let s = Style {
                fg_color: StyleColor::Rgb(RgbColor {
                    r: (i >> 16) as u8,
                    g: (i >> 8) as u8,
                    b: i as u8,
                }),
                ..Style::default()
            };
            t.style(&s, None);
        }
        assert!(t.over_limit());
        t.compact();
        assert_eq!(t.gen(), 1);
        assert!(!t.over_limit());
        assert_eq!((t.styles().len(), t.links().len()), (1, 1));
        assert_eq!(t.link("x"), 1);
    }
}
