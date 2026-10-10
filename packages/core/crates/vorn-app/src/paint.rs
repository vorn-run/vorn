//! Paints a pane's cell grid into a card's body: background runs and
//! decorations as quads, then each cell's glyph from the cell cache, all on
//! whole pixels.

use crate::ui::{Rect, Rgba, Ui, BOLD, ITALIC};
use crate::view::{flags, Cursor, PaneView};

/// Draws `v` in `r` (physical pixels). `focused` draws the cursor solid;
/// `preedit` is IME composition shown at the cursor.
pub fn pane(ui: &mut Ui, v: &PaneView, r: Rect, focused: bool, preedit: &str) {
    let cell = ui.text.cell();
    let (x0, y0) = (r.x.round(), r.y.round());
    let at = |col: u16, row: u16| (x0 + f32::from(col) * cell.w, y0 + f32::from(row) * cell.h);
    let thin = ui.scale().round().max(1.0);
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
    let cursor = (v.cursor_visible && preedit.is_empty()).then_some((v.cursor_x, v.cursor_y));
    // An unfocused terminal shows a hollow cursor, as the renderer's does.
    let shape = if focused { v.cursor } else { Cursor::Hollow };
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
                &mut ui.renderer,
                &ui.gpu.queue,
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
                &mut ui.renderer,
                &ui.gpu.queue,
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
            &mut ui.renderer,
            &ui.gpu.queue,
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
