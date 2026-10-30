//! Block elements and box drawing as geometry that fills the cell, so neighbours tile without seams.

/// A stroke's weight on one arm of a box drawing character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W {
    N,
    L,
    H,
    D,
}

use W::{D, H, L, N};

/// A cell-sized coverage mask, row by row.
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        Canvas {
            w: w as i32,
            h: h as i32,
            px: vec![0; w as usize * h as usize],
        }
    }

    /// Covers `[x0, x1) × [y0, y1)`, clipped to the cell, at `a`.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, a: u8) {
        let (x0, x1) = (x0.clamp(0, self.w), x1.clamp(0, self.w));
        let (y0, y1) = (y0.clamp(0, self.h), y1.clamp(0, self.h));
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            for p in &mut self.px[row + x0 as usize..row + x1 as usize] {
                *p = (*p).max(a);
            }
        }
    }

    /// Strokes the polyline `pts` `width` pixels wide, antialiased by 4×4 samples a pixel.
    fn stroke(&mut self, pts: &[(f32, f32)], width: f32) {
        const S: usize = 4;
        let r2 = (width / 2.0) * (width / 2.0);
        for y in 0..self.h {
            for x in 0..self.w {
                let mut hit = 0;
                for sy in 0..S {
                    for sx in 0..S {
                        let p = (
                            x as f32 + (sx as f32 + 0.5) / S as f32,
                            y as f32 + (sy as f32 + 0.5) / S as f32,
                        );
                        if pts.windows(2).any(|s| dist2(p, s[0], s[1]) <= r2) {
                            hit += 1;
                        }
                    }
                }
                let a = (hit * 255 / (S * S)) as u8;
                let p = &mut self.px[(y * self.w + x) as usize];
                *p = (*p).max(a);
            }
        }
    }
}

/// The squared distance from `p` to the segment `a`–`b`.
fn dist2(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (ex, ey) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    ex * ex + ey * ey
}

/// `n` eighths of `size`, rounded to a whole pixel.
fn eighths(n: i32, size: i32) -> i32 {
    (n * size + 4) / 8
}

/// `ch`'s coverage in a `w` × `h` pixel cell, light lines `stroke` thick; `None` leaves it to the font.
pub fn mask(ch: char, w: u32, h: u32, stroke: u32) -> Option<Vec<u8>> {
    let cp = u32::from(ch);
    if !(0x2500..=0x259f).contains(&cp) || w == 0 || h == 0 {
        return None;
    }
    let mut c = Canvas::new(w, h);
    let t = stroke.max(1) as i32;
    if cp >= 0x2580 {
        block(&mut c, cp);
    } else if let Some(arms) = lines(cp) {
        draw_lines(&mut c, arms, t);
    } else {
        match cp {
            0x2504 => dash_h(&mut c, 3, t),
            0x2505 => dash_h(&mut c, 3, 2 * t),
            0x2506 => dash_v(&mut c, 3, t),
            0x2507 => dash_v(&mut c, 3, 2 * t),
            0x2508 => dash_h(&mut c, 4, t),
            0x2509 => dash_h(&mut c, 4, 2 * t),
            0x250a => dash_v(&mut c, 4, t),
            0x250b => dash_v(&mut c, 4, 2 * t),
            0x254c => dash_h(&mut c, 2, t),
            0x254d => dash_h(&mut c, 2, 2 * t),
            0x254e => dash_v(&mut c, 2, t),
            0x254f => dash_v(&mut c, 2, 2 * t),
            0x256d..=0x2570 => arc(&mut c, cp, t),
            0x2571 => diagonal(&mut c, false, t),
            0x2572 => diagonal(&mut c, true, t),
            0x2573 => {
                diagonal(&mut c, false, t);
                diagonal(&mut c, true, t);
            }
            _ => return None,
        }
    }
    Some(c.px)
}

/// U+2580–U+259F; every half splits at one pixel, so complements never overlap or gap.
fn block(c: &mut Canvas, cp: u32) {
    let (w, h) = (c.w, c.h);
    let (mx, my) = (eighths(4, w), eighths(4, h));
    let quad = |c: &mut Canvas, tl: bool, tr: bool, bl: bool, br: bool| {
        for (on, x0, y0, x1, y1) in [
            (tl, 0, 0, mx, my),
            (tr, mx, 0, w, my),
            (bl, 0, my, mx, h),
            (br, mx, my, w, h),
        ] {
            if on {
                c.rect(x0, y0, x1, y1, 255);
            }
        }
    };
    match cp {
        0x2580 => c.rect(0, 0, w, my, 255),
        0x2581..=0x2587 => c.rect(0, eighths(0x2588 - cp as i32, h), w, h, 255),
        0x2588 => c.rect(0, 0, w, h, 255),
        0x2589..=0x258f => c.rect(0, 0, eighths(0x2590 - cp as i32, w), h, 255),
        0x2590 => c.rect(mx, 0, w, h, 255),
        0x2591 => c.rect(0, 0, w, h, 0x40),
        0x2592 => c.rect(0, 0, w, h, 0x80),
        0x2593 => c.rect(0, 0, w, h, 0xc0),
        0x2594 => c.rect(0, 0, w, eighths(1, h), 255),
        0x2595 => c.rect(w - eighths(1, w), 0, w, h, 255),
        0x2596 => quad(c, false, false, true, false),
        0x2597 => quad(c, false, false, false, true),
        0x2598 => quad(c, true, false, false, false),
        0x2599 => quad(c, true, false, true, true),
        0x259a => quad(c, true, false, false, true),
        0x259b => quad(c, true, true, true, false),
        0x259c => quad(c, true, true, false, true),
        0x259d => quad(c, false, true, false, false),
        0x259e => quad(c, false, true, true, false),
        _ => quad(c, false, true, true, true),
    }
}

/// The arms (up, right, down, left) of a box drawing character made of straight lines.
fn lines(cp: u32) -> Option<[W; 4]> {
    Some(match cp {
        0x2500 => [N, L, N, L],
        0x2501 => [N, H, N, H],
        0x2502 => [L, N, L, N],
        0x2503 => [H, N, H, N],
        0x250c => [N, L, L, N],
        0x250d => [N, H, L, N],
        0x250e => [N, L, H, N],
        0x250f => [N, H, H, N],
        0x2510 => [N, N, L, L],
        0x2511 => [N, N, L, H],
        0x2512 => [N, N, H, L],
        0x2513 => [N, N, H, H],
        0x2514 => [L, L, N, N],
        0x2515 => [L, H, N, N],
        0x2516 => [H, L, N, N],
        0x2517 => [H, H, N, N],
        0x2518 => [L, N, N, L],
        0x2519 => [L, N, N, H],
        0x251a => [H, N, N, L],
        0x251b => [H, N, N, H],
        0x251c => [L, L, L, N],
        0x251d => [L, H, L, N],
        0x251e => [H, L, L, N],
        0x251f => [L, L, H, N],
        0x2520 => [H, L, H, N],
        0x2521 => [H, H, L, N],
        0x2522 => [L, H, H, N],
        0x2523 => [H, H, H, N],
        0x2524 => [L, N, L, L],
        0x2525 => [L, N, L, H],
        0x2526 => [H, N, L, L],
        0x2527 => [L, N, H, L],
        0x2528 => [H, N, H, L],
        0x2529 => [H, N, L, H],
        0x252a => [L, N, H, H],
        0x252b => [H, N, H, H],
        0x252c => [N, L, L, L],
        0x252d => [N, L, L, H],
        0x252e => [N, H, L, L],
        0x252f => [N, H, L, H],
        0x2530 => [N, L, H, L],
        0x2531 => [N, L, H, H],
        0x2532 => [N, H, H, L],
        0x2533 => [N, H, H, H],
        0x2534 => [L, L, N, L],
        0x2535 => [L, L, N, H],
        0x2536 => [L, H, N, L],
        0x2537 => [L, H, N, H],
        0x2538 => [H, L, N, L],
        0x2539 => [H, L, N, H],
        0x253a => [H, H, N, L],
        0x253b => [H, H, N, H],
        0x253c => [L, L, L, L],
        0x253d => [L, L, L, H],
        0x253e => [L, H, L, L],
        0x253f => [L, H, L, H],
        0x2540 => [H, L, L, L],
        0x2541 => [L, L, H, L],
        0x2542 => [H, L, H, L],
        0x2543 => [H, L, L, H],
        0x2544 => [H, H, L, L],
        0x2545 => [L, L, H, H],
        0x2546 => [L, H, H, L],
        0x2547 => [H, H, L, H],
        0x2548 => [L, H, H, H],
        0x2549 => [H, L, H, H],
        0x254a => [H, H, H, L],
        0x254b => [H, H, H, H],
        0x2550 => [N, D, N, D],
        0x2551 => [D, N, D, N],
        0x2552 => [N, D, L, N],
        0x2553 => [N, L, D, N],
        0x2554 => [N, D, D, N],
        0x2555 => [N, N, L, D],
        0x2556 => [N, N, D, L],
        0x2557 => [N, N, D, D],
        0x2558 => [L, D, N, N],
        0x2559 => [D, L, N, N],
        0x255a => [D, D, N, N],
        0x255b => [L, N, N, D],
        0x255c => [D, N, N, L],
        0x255d => [D, N, N, D],
        0x255e => [L, D, L, N],
        0x255f => [D, L, D, N],
        0x2560 => [D, D, D, N],
        0x2561 => [L, N, L, D],
        0x2562 => [D, N, D, L],
        0x2563 => [D, N, D, D],
        0x2564 => [N, D, L, D],
        0x2565 => [N, L, D, L],
        0x2566 => [N, D, D, D],
        0x2567 => [L, D, N, D],
        0x2568 => [D, L, N, L],
        0x2569 => [D, D, N, D],
        0x256a => [L, D, L, D],
        0x256b => [D, L, D, L],
        0x256c => [D, D, D, D],
        0x2574 => [N, N, N, L],
        0x2575 => [L, N, N, N],
        0x2576 => [N, L, N, N],
        0x2577 => [N, N, L, N],
        0x2578 => [N, N, N, H],
        0x2579 => [H, N, N, N],
        0x257a => [N, H, N, N],
        0x257b => [N, N, H, N],
        0x257c => [N, H, N, L],
        0x257d => [L, N, H, N],
        0x257e => [N, L, N, H],
        0x257f => [H, N, L, N],
        _ => return None,
    })
}

/// Each arm from its edge to where it meets the others: light `t` thick, heavy `2t`, double two light `t` apart.
fn draw_lines(c: &mut Canvas, [up, right, down, left]: [W; 4], t: i32) {
    let (w, h) = (c.w, c.h);
    let (hl_top, hh_top) = ((h - t).max(0) / 2, (h - 2 * t).max(0) / 2);
    let (hl_bot, hh_bot) = (hl_top + t, hh_top + 2 * t);
    let (hd_top, hd_bot) = (hl_top - t, hl_bot + t);
    let (vl_left, vh_left) = ((w - t).max(0) / 2, (w - 2 * t).max(0) / 2);
    let (vl_right, vh_right) = (vl_left + t, vh_left + 2 * t);
    let (vd_left, vd_right) = (vl_left - t, vl_right + t);

    // Where a vertical arm stops: past the strokes across it, so corners close.
    let across =
        |a: W, b: W, along_a: W, along_b: W, light: i32, heavy: i32, double: i32, inner: i32| {
            if a == H || b == H {
                heavy
            } else if a != b || along_a == along_b {
                if a == D || b == D {
                    double
                } else {
                    light
                }
            } else if a == N && b == N {
                light
            } else {
                inner
            }
        };
    let up_bottom = across(left, right, down, up, hl_bot, hh_bot, hd_bot, hl_top);
    let down_top = across(left, right, up, down, hl_top, hh_top, hd_top, hl_bot);
    let left_right = across(up, down, left, right, vl_right, vh_right, vd_right, vl_left);
    let right_left = across(up, down, right, left, vl_left, vh_left, vd_left, vl_right);

    match up {
        N => {}
        L => c.rect(vl_left, 0, vl_right, up_bottom, 255),
        H => c.rect(vh_left, 0, vh_right, up_bottom, 255),
        D => {
            let lb = if left == D { hl_top } else { up_bottom };
            let rb = if right == D { hl_top } else { up_bottom };
            c.rect(vd_left, 0, vl_left, lb, 255);
            c.rect(vl_right, 0, vd_right, rb, 255);
        }
    }
    match right {
        N => {}
        L => c.rect(right_left, hl_top, w, hl_bot, 255),
        H => c.rect(right_left, hh_top, w, hh_bot, 255),
        D => {
            let tl = if up == D { vl_right } else { right_left };
            let bl = if down == D { vl_right } else { right_left };
            c.rect(tl, hd_top, w, hl_top, 255);
            c.rect(bl, hl_bot, w, hd_bot, 255);
        }
    }
    match down {
        N => {}
        L => c.rect(vl_left, down_top, vl_right, h, 255),
        H => c.rect(vh_left, down_top, vh_right, h, 255),
        D => {
            let lt = if left == D { hl_bot } else { down_top };
            let rt = if right == D { hl_bot } else { down_top };
            c.rect(vd_left, lt, vl_left, h, 255);
            c.rect(vl_right, rt, vd_right, h, 255);
        }
    }
    match left {
        N => {}
        L => c.rect(0, hl_top, left_right, hl_bot, 255),
        H => c.rect(0, hh_top, left_right, hh_bot, 255),
        D => {
            let tr = if up == D { vl_left } else { left_right };
            let br = if down == D { vl_left } else { left_right };
            c.rect(0, hd_top, tr, hl_top, 255);
            c.rect(0, hl_bot, br, hd_bot, 255);
        }
    }
}

/// `n` dashes `t` thick across the cell, half a gap at each end so a row of them reads as one line.
fn dash_h(c: &mut Canvas, n: i32, t: i32) {
    let w = c.w;
    let y = (c.h - t).max(0) / 2;
    if w < 2 * n {
        c.rect(0, y, w, y + t, 255);
        return;
    }
    let gap = 4.max(t).min(w / (2 * n));
    let dash = (w - n * gap) / n;
    let mut extra = (w - n * gap) % n;
    let mut x = gap / 2;
    for _ in 0..n {
        let x1 = x + dash + i32::from(extra > 0);
        extra -= 1;
        c.rect(x, y, x1, y + t, 255);
        x = x1 + gap;
    }
}

/// `n` dashes `t` thick down the cell, the gap after each so a column of them reads as one line.
fn dash_v(c: &mut Canvas, n: i32, t: i32) {
    let h = c.h;
    let x = (c.w - t).max(0) / 2;
    if h < 2 * n {
        c.rect(x, 0, x + t, h, 255);
        return;
    }
    let gap = 4.max(t).min(h / (2 * n));
    let dash = (h - n * gap) / n;
    let mut extra = (h - n * gap) % n;
    let mut y = 0;
    for _ in 0..n {
        let y1 = y + dash + i32::from(extra > 0);
        extra -= 1;
        c.rect(x, y, x + t, y1, 255);
        y = y1 + gap;
    }
}

/// U+256D–U+2570: a light line that turns the corner on a curve, centred on the straight lines it joins.
fn arc(c: &mut Canvas, cp: u32, t: i32) {
    let (w, h, tf) = (c.w as f32, c.h as f32, t as f32);
    let cx = ((c.w - t).max(0) / 2) as f32 + tf / 2.0;
    let cy = ((c.h - t).max(0) / 2) as f32 + tf / 2.0;
    let r = w.min(h) / 2.0;
    // Which way the vertical arm goes (down: +1) and the horizontal one (right: +1).
    let (vy, hx) = match cp {
        0x256d => (1.0, 1.0),
        0x256e => (1.0, -1.0),
        0x256f => (-1.0, -1.0),
        _ => (-1.0, 1.0),
    };
    let edge_y = if vy > 0.0 { h } else { 0.0 };
    let edge_x = if hx > 0.0 { w } else { 0.0 };
    let (p0, p3) = ((cx, cy + vy * r), (cx + hx * r, cy));
    let (p1, p2) = ((cx, cy + vy * r * 0.25), (cx + hx * r * 0.25, cy));
    let mut pts = Vec::with_capacity(20);
    pts.push((cx, edge_y));
    for i in 0..=16 {
        let s = i as f32 / 16.0;
        let u = 1.0 - s;
        let (a, b, cc, d) = (u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s);
        pts.push((
            a * p0.0 + b * p1.0 + cc * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + cc * p2.1 + d * p3.1,
        ));
    }
    pts.push((edge_x, cy));
    c.stroke(&pts, tf);
}

/// U+2571/U+2572: a light diagonal from corner to corner, overshooting each a little so they tile.
fn diagonal(c: &mut Canvas, down: bool, t: i32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let (sx, sy) = ((w / h).min(1.0) * 0.5, (h / w).min(1.0) * 0.5);
    let pts = if down {
        [(-sx, -sy), (w + sx, h + sy)]
    } else {
        [(w + sx, -sy), (-sx, h + sy)]
    };
    c.stroke(&pts, t as f32);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ch` in a 14 × 29 cell with 2 px lines, as rows of `#` (covered) and `.`.
    fn rows(ch: char) -> Vec<String> {
        let m = mask(ch, 14, 29, 2).expect("drawn");
        m.chunks(14)
            .map(|r| {
                r.iter()
                    .map(|&a| if a == 255 { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    fn covered(ch: char) -> Vec<bool> {
        mask(ch, 14, 29, 2)
            .expect("drawn")
            .iter()
            .map(|&a| a == 255)
            .collect()
    }

    #[test]
    fn only_blocks_and_box_drawing_are_drawn() {
        assert!(mask('a', 14, 29, 2).is_none());
        assert!(mask('\u{24ff}', 14, 29, 2).is_none());
        assert!(mask('\u{25a0}', 14, 29, 2).is_none());
        assert!(mask('█', 0, 29, 2).is_none());
    }

    #[test]
    fn the_full_block_fills_every_pixel() {
        assert!(covered('█').iter().all(|&c| c));
    }

    #[test]
    fn halves_and_their_complements_tile_without_overlap() {
        for (a, b) in [
            ('▀', '▄'),
            ('▌', '▐'),
            ('▘', '▟'),
            ('▝', '▙'),
            ('▖', '▜'),
            ('▗', '▛'),
            ('▚', '▞'),
        ] {
            let (a, b) = (covered(a), covered(b));
            assert!(a.iter().zip(&b).all(|(x, y)| x != y), "{a:?} {b:?}");
        }
    }

    #[test]
    fn quadrants_split_where_the_halves_do() {
        let r = rows('▘');
        // 29 rows split at 15 (14.5 rounded), 14 columns at 7.
        assert_eq!(r[14], "#######.......");
        assert_eq!(r[15], "..............");
        assert_eq!(rows('▀')[14], "##############");
        assert_eq!(rows('▌')[28], "#######.......");
    }

    #[test]
    fn eighths_grow_from_their_edge() {
        let lower: Vec<usize> = ('▁'..='▇')
            .map(|c| covered(c).iter().filter(|&&x| x).count() / 14)
            .collect();
        assert_eq!(lower, [4, 7, 11, 14, 18, 22, 25]);
        assert_eq!(rows('▏')[0], "##............");
        assert_eq!(rows('▕')[0], "............##");
        assert_eq!(rows('▔')[3], "##############");
        assert_eq!(rows('▔')[4], "..............");
    }

    #[test]
    fn shades_cover_the_cell_partly() {
        for (ch, a) in [('░', 0x40), ('▒', 0x80), ('▓', 0xc0)] {
            assert!(mask(ch, 14, 29, 2).expect("drawn").iter().all(|&p| p == a));
        }
    }

    #[test]
    fn straight_lines_run_edge_to_edge_through_the_centre() {
        let r = rows('─');
        assert_eq!(
            &r[12..16],
            [
                "..............",
                "##############",
                "##############",
                ".............."
            ]
        );
        let r = rows('│');
        assert!(r.iter().all(|row| row == "......##......"));
        // Heavy is twice as thick.
        assert_eq!(rows('━').iter().filter(|r| r.starts_with('#')).count(), 4);
    }

    #[test]
    fn a_corner_closes_without_a_notch() {
        let r = rows('┌');
        assert_eq!(r[13], "......########");
        assert_eq!(r[14], "......########");
        assert_eq!(r[28], "......##......");
        assert_eq!(r[0], "..............");
        let r = rows('┘');
        assert_eq!(r[0], "......##......");
        assert_eq!(r[14], "########......");
    }

    #[test]
    fn neighbouring_lines_meet_at_the_cell_edge() {
        // Each arm reaches the edge it points at, so the next cell's line continues it.
        for ch in ['─', '┼', '┬', '╋', '═', '╬', '┄', '╌'] {
            let r = rows(ch);
            let at = |x: usize| r.iter().map(|row| row.as_bytes()[x]).collect::<Vec<u8>>();
            assert_eq!(at(0), at(13), "{ch}");
        }
        for ch in ['│', '┼', '├', '║', '╬'] {
            let r = rows(ch);
            assert_eq!(r[0], r[28], "{ch}");
        }
    }

    #[test]
    fn double_lines_are_two_strokes_with_a_gap() {
        let r = rows('║');
        assert!(r.iter().all(|row| row == "....##..##...."));
        let r = rows('╔');
        assert_eq!(r[11], "....##########");
        assert_eq!(r[13], "....##........");
        assert_eq!(r[15], "....##..######");
        assert_eq!(r[28], "....##..##....");
    }

    #[test]
    fn dashes_leave_gaps_and_fill_the_rest() {
        let r = rows('┄');
        assert_eq!(r[13], ".###..###..##.");
        let r = rows('┆');
        let col: String = r.iter().map(|row| &row[6..7]).collect();
        assert_eq!(col, "######....######....#####....");
    }

    #[test]
    fn arcs_and_diagonals_reach_the_lines_they_join() {
        let m = mask('╭', 14, 29, 2).expect("drawn");
        let at = |x: usize, y: usize| m[y * 14 + x];
        // The vertical arm leaves the bottom edge, the horizontal one the right edge, centred.
        assert_eq!((at(6, 28), at(7, 28)), (255, 255));
        assert_eq!((at(13, 13), at(13, 14)), (255, 255));
        assert_eq!(at(0, 0), 0);
        let d = mask('╲', 14, 29, 2).expect("drawn");
        assert!(d[0] > 0 && d[14 * 29 - 1] > 0 && d[13] == 0);
    }
}
