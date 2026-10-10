//! Paints a pane's cell grid with GPUI: background runs as quads, each
//! style run shaped at the grid's advance, the cursor and IME preedit.

use gpui::{
    fill, outline, point, px, rgb, size, App, BorderStyle, Bounds, Font, FontStyle, FontWeight,
    Hsla, Pixels, SharedString, StrikethroughStyle, TextAlign, TextRun, UnderlineStyle, Window,
};
use spike_shared::view::flags;
use spike_shared::PaneView;

/// The terminal font and its cell.
pub struct Metrics {
    pub font: Font,
    pub size: Pixels,
    pub cell: gpui::Size<Pixels>,
}

fn c(v: u32) -> Hsla {
    rgb(v).into()
}

fn run(len: usize, font: &Font, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Draws `v` in `bounds`; `preedit` is IME composition shown at the cursor.
pub fn paint_pane(
    bounds: Bounds<Pixels>,
    v: &PaneView,
    m: &Metrics,
    preedit: &str,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, c(v.bg)));
    let (cw, ch) = (m.cell.width, m.cell.height);
    let o = bounds.origin;
    let at = |col: u16, row: u16| point(o.x + cw * f32::from(col), o.y + ch * f32::from(row));
    for r in &v.runs {
        if r.bg != v.bg {
            window.paint_quad(fill(
                Bounds::new(at(r.col, r.row), size(cw * f32::from(r.ncols), ch)),
                c(r.bg),
            ));
        }
    }
    let cursor = (v.cursor_visible && preedit.is_empty()).then_some((v.cursor_x, v.cursor_y));
    if let Some((x, y)) = cursor {
        let p = at(x, y);
        let cc = c(v.cursor_color);
        let b = match v.cursor_style {
            2 => Bounds::new(p, size(px(2.0), ch)),
            3 => Bounds::new(point(p.x, p.y + ch - px(2.0)), size(cw, px(2.0))),
            _ => Bounds::new(p, size(cw, ch)),
        };
        if v.cursor_style == 1 {
            window.paint_quad(outline(b, cc, BorderStyle::Solid));
        } else {
            window.paint_quad(fill(b, cc));
        }
    }
    let block_at = cursor.filter(|_| v.cursor_style == 0);
    let ts = window.text_system().clone();
    for r in &v.runs {
        let text = v.run_text(r);
        let decorated = r.flags & (flags::UNDERLINE | flags::STRIKE) != 0;
        if text.trim().is_empty() && !decorated {
            continue;
        }
        let mut f = m.font.clone();
        if r.flags & flags::BOLD != 0 {
            f.weight = FontWeight::BOLD;
        }
        if r.flags & flags::ITALIC != 0 {
            f.style = FontStyle::Italic;
        }
        let mut fg = c(r.fg);
        if r.flags & flags::FAINT != 0 {
            fg.a = 0.5;
        }
        let mut runs = vec![run(text.len(), &f, fg)];
        // The block cursor's cell is drawn in the background color.
        if let Some((x, y)) = block_at {
            let plain = r.flags & flags::CLUSTER == 0;
            if y == r.row && x >= r.col && x < r.col + r.ncols && plain {
                let i = usize::from(x - r.col);
                runs = [(0..i, fg), (i..i + 1, c(v.bg)), (i + 1..text.len(), fg)]
                    .into_iter()
                    .filter(|(range, _)| !range.is_empty())
                    .map(|(range, color)| run(range.len(), &f, color))
                    .collect();
            } else if y == r.row && x == r.col {
                runs[0].color = c(v.bg);
            }
        }
        for tr in &mut runs {
            tr.underline = (r.flags & flags::UNDERLINE != 0).then_some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(fg),
                wavy: false,
            });
            tr.strikethrough = (r.flags & flags::STRIKE != 0).then_some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(fg),
            });
        }
        // Plain runs keep the grid's advance; clusters are drawn alone.
        let force = (r.flags & flags::CLUSTER == 0).then_some(cw);
        let line = ts.shape_line(SharedString::from(text.to_owned()), m.size, &runs, force);
        let _ = line.paint(at(r.col, r.row), ch, TextAlign::Left, None, window, cx);
    }
    if !preedit.is_empty() {
        let p = at(v.cursor_x, v.cursor_y);
        let mut tr = run(preedit.len(), &m.font, c(v.fg));
        tr.background_color = Some(c(v.bg));
        tr.underline = Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(c(v.fg)),
            wavy: false,
        });
        let line = ts.shape_line(SharedString::from(preedit.to_owned()), m.size, &[tr], None);
        let _ = line.paint(p, ch, TextAlign::Left, None, window, cx);
    }
}
