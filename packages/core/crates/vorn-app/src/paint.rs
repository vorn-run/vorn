//! Paints a pane's cell grid into a card's body: background runs and
//! decorations as quads, then each cell's glyph from the cell cache, all on
//! whole pixels.

use crate::ui::{Rect, Rgba, Ui, BOLD, ITALIC};
use crate::view::{flags, Cursor, PaneView};

/// How a pane shows its cursor, as today's terminals do: the session being
/// typed into draws its own shape, the current one with keys elsewhere an
/// outline, and the rest none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caret {
    Live,
    Hollow,
    Hidden,
}

/// Draws `v` in `r` (physical pixels) with its cursor shown as `caret`,
/// `dy` whole pixels down from the top while scrolling; `preedit` is IME
/// composition shown at the cursor.
/// `sel` is what is selected: `(row, from col, to col exclusive)` per row.
pub fn pane(
    ui: &mut Ui,
    v: &PaneView,
    r: Rect,
    caret: Caret,
    preedit: &str,
    sel: &[(u16, u16, u16)],
    dy: f32,
) {
    let cell = ui.text.cell();
    let (x0, y0) = (r.x.round(), r.y.round() + dy);
    let at = |col: u16, row: u16| (x0 + f32::from(col) * cell.w, y0 + f32::from(row) * cell.h);
    let thin = ui.scale().round().max(1.0);
    // What a scroll uncovers above or below the rows is the screen's background.
    let bottom = r.y.round() + f32::from(v.rows) * cell.h;
    if dy > 0.0 {
        ui.scene.quad(
            Rect::new(x0, r.y.round(), r.w, dy),
            Rgba::hex(v.bg),
            0.0,
            None,
        );
    } else if dy < 0.0 {
        ui.scene.quad(
            Rect::new(x0, bottom + dy, r.w, -dy),
            Rgba::hex(v.bg),
            0.0,
            None,
        );
    }
    for run in &v.runs {
        let (x, y) = at(run.col, run.row);
        let w = f32::from(run.ncols) * cell.w;
        if run.bg != v.bg {
            ui.scene
                .quad(Rect::new(x, y, w, cell.h), Rgba::hex(run.bg), 0.0, None);
        }
        if run.flags & flags::UNDERLINE != 0 {
            let uy = y + cell.baseline + thin;
            ui.scene
                .quad(Rect::new(x, uy, w, thin), Rgba::hex(run.fg), 0.0, None);
        }
        if run.flags & flags::STRIKE != 0 {
            let sy = y + (cell.h / 2.0).round();
            ui.scene
                .quad(Rect::new(x, sy, w, thin), Rgba::hex(run.fg), 0.0, None);
        }
    }
    let sel_bg = Rgba::hex(crate::look::term::SELECTION);
    for &(row, a, b) in sel {
        let (x, y) = at(a, row);
        let w = f32::from(b - a) * cell.w;
        ui.scene.quad(Rect::new(x, y, w, cell.h), sel_bg, 0.0, None);
    }
    let cursor = (caret != Caret::Hidden && v.cursor_visible && preedit.is_empty())
        .then_some((v.cursor_x, v.cursor_y));
    let shape = if caret == Caret::Live {
        v.cursor
    } else {
        Cursor::Hollow
    };
    if let Some((cx, cy)) = cursor {
        let (x, y) = at(cx, cy);
        let cc = Rgba::hex(v.cursor_color);
        let rect = match shape {
            Cursor::Bar => Rect::new(x, y, thin * 2.0, cell.h),
            Cursor::Underline => Rect::new(x, y + cell.h - thin * 2.0, cell.w, thin * 2.0),
            Cursor::Block | Cursor::Hollow => Rect::new(x, y, cell.w, cell.h),
        };
        match shape {
            Cursor::Hollow => ui.scene.quad(rect, Rgba::default(), 0.0, Some((thin, cc))),
            _ => ui.scene.quad(rect, cc, 0.0, None),
        }
    }
    let block_at = cursor.filter(|_| shape == Cursor::Block);
    for run in &v.runs {
        let style = (run.flags & (flags::BOLD | flags::ITALIC)) as u8;
        debug_assert_eq!((flags::BOLD as u8, flags::ITALIC as u8), (BOLD, ITALIC));
        let mut fg = Rgba::hex(run.fg);
        if run.flags & flags::FAINT != 0 {
            fg = fg.alpha(0.5);
        }
        let text = v.run_text(run);
        let color_at = |col: u16| match block_at {
            Some((cx, cy)) if cx == col && cy == run.row => Rgba::hex(v.bg),
            _ => fg,
        };
        if run.flags & flags::CLUSTER != 0 {
            let (x, y) = at(run.col, run.row);
            ui.text.draw_cell(
                &mut ui.scene,
                &mut ui.atlases,
                text,
                style,
                (x, y),
                color_at(run.col),
            );
            continue;
        }
        // Runs without CLUSTER are ASCII, one byte per cell.
        for (i, ch) in text.char_indices() {
            if ch == ' ' {
                continue;
            }
            let col = run.col + i as u16;
            let (x, y) = at(col, run.row);
            ui.text.draw_cell(
                &mut ui.scene,
                &mut ui.atlases,
                &text[i..i + ch.len_utf8()],
                style,
                (x, y),
                color_at(col),
            );
        }
    }
    if !preedit.is_empty() {
        // A layer of its own so it covers the glyphs under it.
        ui.scene.layer();
        draw_preedit(ui, v, (x0, y0), preedit);
    }
}

/// Draws `v` into `r` (cells) on a cells surface: the session's own cells
/// copied straight in, and the terminal's real cursor at the session cursor.
pub fn pane_cells(ui: &mut Ui, v: &PaneView, r: Rect, caret: Caret) {
    use crate::ui::cells::{attr, Cell, CursorAt, CursorShape, Op, Sym};
    let rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    let (cols, rows) = (v.cols.min(r.w as u16), v.rows.min(r.h as u16));
    if cols == 0 || rows == 0 {
        return;
    }
    let blank = Cell {
        fg: rgb(v.fg),
        ..Cell::blank(rgb(v.bg))
    };
    let mut cells = vec![blank; usize::from(cols) * usize::from(rows)];
    for run in &v.runs {
        if run.row >= rows {
            continue;
        }
        let mut a = 0;
        for (f, c) in [
            (flags::BOLD, attr::BOLD),
            (flags::ITALIC, attr::ITALIC),
            (flags::UNDERLINE, attr::UNDERLINE),
            (flags::STRIKE, attr::STRIKE),
            (flags::FAINT, attr::DIM),
        ] {
            if run.flags & f != 0 {
                a |= c;
            }
        }
        let cell = |sym| Cell {
            sym,
            fg: rgb(run.fg),
            bg: rgb(run.bg),
            attr: a,
        };
        let row = usize::from(run.row) * usize::from(cols);
        let text = v.run_text(run);
        if run.flags & flags::CLUSTER != 0 {
            for i in 0..run.ncols.min(cols.saturating_sub(run.col)) {
                let sym = if i == 0 { Sym::new(text) } else { Sym::TAIL };
                cells[row + usize::from(run.col + i)] = cell(sym);
            }
            continue;
        }
        for (n, (i, ch)) in text.char_indices().take(usize::from(run.ncols)).enumerate() {
            let col = run.col as usize + n;
            if col < usize::from(cols) {
                cells[row + col] = cell(Sym::new(&text[i..i + ch.len_utf8()]));
            }
        }
    }
    let rect = Rect::new(r.x, r.y, f32::from(cols), f32::from(rows));
    let Some(list) = ui.paint_list_mut() else {
        return;
    };
    list.push_app(Op::Cells { cols, cells }, rect, r);
    if caret == Caret::Live && v.cursor_visible && v.cursor_x < cols && v.cursor_y < rows {
        list.cursor = Some(CursorAt {
            col: (r.x + f32::from(v.cursor_x)) as u16,
            row: (r.y + f32::from(v.cursor_y)) as u16,
            shape: match v.cursor {
                Cursor::Bar => CursorShape::Bar,
                Cursor::Underline => CursorShape::Underline,
                Cursor::Block | Cursor::Hollow => CursorShape::Block,
            },
        });
    }
}

/// IME composition: drawn over the cursor's cells, underlined, the way a
/// terminal shows text that is not yet sent.
fn draw_preedit(ui: &mut Ui, v: &PaneView, origin: (f32, f32), s: &str) {
    let cell = ui.text.cell();
    let mut x = origin.0 + f32::from(v.cursor_x) * cell.w;
    let y = origin.1 + f32::from(v.cursor_y) * cell.h;
    let fg = Rgba::hex(v.fg);
    let start = x;
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        let w = if ch.is_ascii() { 1.0 } else { 2.0 } * cell.w;
        ui.scene
            .quad(Rect::new(x, y, w, cell.h), Rgba::hex(v.bg), 0.0, None);
        ui.text.draw_cell(
            &mut ui.scene,
            &mut ui.atlases,
            ch.encode_utf8(&mut buf),
            0,
            (x, y),
            fg,
        );
        x += w;
    }
    let thin = ui.scale().round().max(1.0);
    ui.scene.quad(
        Rect::new(start, y + cell.h - thin * 2.0, x - start, thin),
        fg,
        0.0,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cells::Op;
    use crate::view::Run;

    #[test]
    fn wide_bytes_land_one_per_cell() {
        let mut ui = Ui::cells(&crate::ui_config(1.0));
        ui.paint_list_mut().expect("cells").begin(Rgba::default());
        let text = "─┼─".as_bytes().to_vec();
        let v = PaneView {
            rows: 1,
            cols: 4,
            runs: vec![Run {
                ncols: 3,
                text_len: text.len() as u32,
                ..Run::default()
            }],
            text,
            ..PaneView::default()
        };
        pane_cells(&mut ui, &v, Rect::new(0.0, 0.0, 4.0, 1.0), Caret::Hollow);
        let list = ui.paint_list().expect("cells");
        let Some(Op::Cells { cells, .. }) = list.items.last().map(|i| &i.op) else {
            panic!("no cells op");
        };
        let row: String = cells.iter().map(|c| c.sym.as_str()).collect();
        assert_eq!(row.trim_end(), "─┼─");
    }
}
