//! A terminal's screen as VT, read back as text, VT with colours, or HTML.
//!
//! vornd answers `terminal:readScrollback` with the screen and its scrollback
//! as escape sequences. Only the text and SGR (colour and weight) survive
//! here: cursor movement, modes and titles describe a live terminal, not a
//! page of output, so they are dropped.

/// A colour: a palette index or a true colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Index(u8),
    Rgb(u8, u8, u8),
}

/// How a run of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub strike: bool,
}

/// Text drawn one way.
pub type Run = (Style, String);
/// One logical line.
pub type Line = Vec<Run>;

fn apply(style: &mut Style, params: &[u32]) {
    let mut i = 0;
    while i < params.len() {
        let p = params[i];
        i += 1;
        match p {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.inverse = true,
            9 => style.strike = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.inverse = false,
            29 => style.strike = false,
            30..=37 => style.fg = Some(Color::Index((p - 30) as u8)),
            40..=47 => style.bg = Some(Color::Index((p - 40) as u8)),
            90..=97 => style.fg = Some(Color::Index((p - 90 + 8) as u8)),
            100..=107 => style.bg = Some(Color::Index((p - 100 + 8) as u8)),
            39 => style.fg = None,
            49 => style.bg = None,
            38 | 48 => {
                let color = match params.get(i) {
                    Some(5) => params.get(i + 1).map(|n| {
                        i += 2;
                        Color::Index(*n as u8)
                    }),
                    Some(2) => match params.get(i + 1..i + 4) {
                        Some(&[r, g, b]) => {
                            i += 4;
                            Some(Color::Rgb(r as u8, g as u8, b as u8))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if p == 38 {
                    style.fg = color;
                } else {
                    style.bg = color;
                }
            }
            _ => {}
        }
    }
}

/// Splits `vt` into logical lines of styled runs.
pub fn parse(vt: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = vec![Vec::new()];
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = vt.chars().peekable();

    fn flush(lines: &mut [Line], style: Style, text: &mut String) {
        if !text.is_empty() {
            if let Some(line) = lines.last_mut() {
                line.push((style, std::mem::take(text)));
            }
        }
    }

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut body = String::new();
                    let mut last = None;
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            last = Some(n);
                            break;
                        }
                        body.push(n);
                    }
                    if last == Some('m') {
                        flush(&mut lines, style, &mut text);
                        let params: Vec<u32> = if body.is_empty() {
                            vec![0]
                        } else {
                            body.split([';', ':'])
                                .map(|p| p.parse().unwrap_or(0))
                                .collect()
                        };
                        apply(&mut style, &params);
                    }
                }
                Some(']') => {
                    // An OSC ends at BEL or ESC \.
                    while let Some(n) = chars.next() {
                        if n == '\x07' || (n == '\x1b' && chars.peek() == Some(&'\\')) {
                            if n == '\x1b' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' => {
                flush(&mut lines, style, &mut text);
                lines.push(Vec::new());
            }
            '\r' => {}
            c if c.is_control() && c != '\t' => {}
            c => text.push(c),
        }
    }
    flush(&mut lines, style, &mut text);
    lines
}

fn is_blank(line: &Line) -> bool {
    line.iter().all(|(_, t)| t.trim().is_empty())
}

/// Drops trailing spaces on every line and blank lines at the end.
pub fn trim(lines: &mut Vec<Line>) {
    for line in lines.iter_mut() {
        while let Some((_, last)) = line.last_mut() {
            let kept = last.trim_end_matches(' ').len();
            last.truncate(kept);
            if last.is_empty() {
                line.pop();
            } else {
                break;
            }
        }
    }
    while lines.last().is_some_and(is_blank) {
        lines.pop();
    }
}

/// Keeps the last `n` lines.
pub fn tail(lines: &mut Vec<Line>, n: usize) {
    if lines.len() > n {
        lines.drain(..lines.len() - n);
    }
}

/// Plain text, a newline after every line.
pub fn text(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        for (_, t) in line {
            out.push_str(t);
        }
        out.push('\n');
    }
    out
}

/// Text with the SGR sequences that colour it, each line reset at its end.
pub fn vt(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        let mut styled = false;
        for (s, t) in line {
            if *s == Style::default() {
                if styled {
                    out.push_str("\x1b[0m");
                    styled = false;
                }
            } else {
                out.push_str("\x1b[0");
                for (on, code) in [
                    (s.bold, "1"),
                    (s.dim, "2"),
                    (s.italic, "3"),
                    (s.underline, "4"),
                    (s.inverse, "7"),
                    (s.strike, "9"),
                ] {
                    if on {
                        out.push(';');
                        out.push_str(code);
                    }
                }
                for (color, base) in [(s.fg, 38), (s.bg, 48)] {
                    match color {
                        Some(Color::Index(n)) => out.push_str(&format!(";{base};5;{n}")),
                        Some(Color::Rgb(r, g, b)) => {
                            out.push_str(&format!(";{base};2;{r};{g};{b}"));
                        }
                        None => {}
                    }
                }
                out.push('m');
                styled = true;
            }
            out.push_str(t);
        }
        if styled {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    out
}

/// The terminal's default 16 colours (xterm's), as `#rrggbb`.
const BASE: [&str; 16] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
    "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
];

/// A colour as `#rrggbb`, with the 256-colour palette's cube and ramp.
pub fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Index(n) if n < 16 => BASE[usize::from(n)].to_owned(),
        Color::Index(n) if n < 232 => {
            let n = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            format!(
                "#{:02x}{:02x}{:02x}",
                level(n / 36),
                level(n / 6 % 6),
                level(n % 6)
            )
        }
        Color::Index(n) => {
            let v = 8 + 10 * (n - 232);
            format!("#{v:02x}{v:02x}{v:02x}")
        }
    }
}

fn escape(t: &str, out: &mut String) {
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// A `<pre>` with each coloured run as a span.
pub fn html(lines: &[Line]) -> String {
    let mut out = String::from("<pre>");
    for line in lines {
        for (s, t) in line {
            let (fg, bg) = if s.inverse {
                (
                    s.bg.or(Some(Color::Index(0))),
                    s.fg.or(Some(Color::Index(7))),
                )
            } else {
                (s.fg, s.bg)
            };
            let mut css = String::new();
            if let Some(c) = fg {
                css.push_str(&format!("color:{};", hex(c)));
            }
            if let Some(c) = bg {
                css.push_str(&format!("background:{};", hex(c)));
            }
            if s.bold {
                css.push_str("font-weight:bold;");
            }
            if s.dim {
                css.push_str("opacity:.6;");
            }
            if s.italic {
                css.push_str("font-style:italic;");
            }
            match (s.underline, s.strike) {
                (true, true) => css.push_str("text-decoration:underline line-through;"),
                (true, false) => css.push_str("text-decoration:underline;"),
                (false, true) => css.push_str("text-decoration:line-through;"),
                _ => {}
            }
            if css.is_empty() {
                escape(t, &mut out);
            } else {
                out.push_str(&format!("<span style=\"{css}\">"));
                escape(t, &mut out);
                out.push_str("</span>");
            }
        }
        out.push('\n');
    }
    out.push_str("</pre>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\x1b[1;31mred\x1b[0m plain  \r\n\x1b[38;2;1;2;3mtc\x1b[0m <&>\r\n\x1b]0;title\x07\x1b[H\r\n\r\n";

    #[test]
    fn keeps_text_and_drops_what_only_a_live_terminal_needs() {
        let lines = parse(SAMPLE);
        assert_eq!(text(&lines), "red plain  \ntc <&>\n\n\n\n");
    }

    #[test]
    fn trims_trailing_spaces_and_blank_lines() {
        let mut lines = parse(SAMPLE);
        trim(&mut lines);
        assert_eq!(text(&lines), "red plain\ntc <&>\n");
        tail(&mut lines, 1);
        assert_eq!(text(&lines), "tc <&>\n");
    }

    #[test]
    fn writes_vt_back_with_its_colours() {
        let mut lines = parse(SAMPLE);
        trim(&mut lines);
        assert_eq!(
            vt(&lines),
            "\x1b[0;1;38;5;1mred\x1b[0m plain\n\x1b[0;38;2;1;2;3mtc\x1b[0m <&>\n"
        );
    }

    #[test]
    fn renders_html_spans_with_the_palette_and_escapes_text() {
        let mut lines = parse(SAMPLE);
        trim(&mut lines);
        assert_eq!(
            html(&lines),
            "<pre><span style=\"color:#cd0000;font-weight:bold;\">red</span> plain\n<span style=\"color:#010203;\">tc</span> &lt;&amp;&gt;\n</pre>\n"
        );
    }

    #[test]
    fn maps_the_256_colour_cube_and_grey_ramp() {
        assert_eq!(hex(Color::Index(16)), "#000000");
        assert_eq!(hex(Color::Index(196)), "#ff0000");
        assert_eq!(hex(Color::Index(231)), "#ffffff");
        assert_eq!(hex(Color::Index(232)), "#080808");
        assert_eq!(hex(Color::Index(9)), "#ff0000");
    }
}
