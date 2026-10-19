//! Frames as kitty images: the first frame is one image, and each frame
//! after it sends only the rectangles of tiles that changed, as images
//! placed over it, until they cover enough that one whole frame is
//! cheaper. Overlays (menus, popovers, tooltips) are an image of their
//! own above everything, so opening and closing one sends nothing under it.
//!
//! Inside tmux images cannot stack: each one is drawn by placeholder cells
//! tmux keeps as text, so there the changed rectangles grow to whole cells
//! and replace the cells they cover.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use super::kitty::{self, Format, Medium, Show, Wrap};

/// Above this many live patches a whole frame is sent instead.
const MAX_PATCHES: usize = 64;
/// More changed rectangles than this in one frame and their bounding box
/// is sent instead: fewer commands for about the same pixels.
const MAX_RECTS: usize = 12;
/// A frame changing at least this share of the screen (in percent) is
/// sent whole.
const WHOLE_PERCENT: u64 = 60;
/// Images at least this large go through a temporary file where the
/// terminal reads them: below it the file costs more than the bytes.
const FILE_MIN: usize = 24 * 1024;
/// Inline images at least this large are deflated where that is on.
const ZLIB_MIN: usize = 2 * 1024;
/// Overlays sit above every patch.
const OVERLAY_Z: i32 = 1 << 30;
/// Image ids stay within 24 bits, what a placeholder's colour carries.
const MAX_ID: u32 = 0x00ff_ffff;

/// A frame's pixels as the CPU renderer drew them: RGBA bytes in memory
/// order, a `u32` each, and which tiles changed since the frame before.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub px: &'a [u32],
    pub size: (u32, u32),
    pub dirty: &'a [bool],
    pub tiles_x: usize,
    pub tile: u32,
}

/// How images are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placing {
    /// At the cursor, stacked by depth.
    Cursor,
    /// By placeholder cells (inside tmux).
    Cells,
}

/// How image bytes travel to the terminal.
#[derive(Debug, Clone, Default)]
pub struct Transport {
    /// The temporary directory the terminal reads files from, when it runs
    /// on this machine and said it reads them.
    pub file_dir: Option<PathBuf>,
    /// Deflate inline images: worth it when the bytes go over a network.
    pub zlib: bool,
}

/// What one frame sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sent {
    /// Bytes written to the terminal (files' pixels not counted).
    pub bytes: usize,
    /// Bytes that went through temporary files instead.
    pub file_bytes: usize,
    pub images: usize,
    /// The whole frame was sent.
    pub whole: bool,
}

/// A rectangle in pixels: `[x0, y0, x1, y1)`.
pub type Rect = [u32; 4];

#[derive(Debug, Clone, Copy)]
struct Patch {
    id: u32,
    rect: Rect,
}

/// Turns frames into the commands that show them, keeping what the
/// terminal already has.
pub struct Presenter {
    wrap: Wrap,
    placing: Placing,
    /// A cell's size in pixels.
    cell: (u32, u32),
    transport: Transport,
    size: (u32, u32),
    base: Option<u32>,
    patches: Vec<Patch>,
    overlay: Option<Patch>,
    next_id: u32,
    next_z: i32,
    /// Placeholder cells: which image each one shows, and how many cells
    /// each image still has (it is deleted at none).
    owners: Vec<u32>,
    counts: HashMap<u32, u32>,
    files: u64,
    scratch: Vec<u8>,
    deflated: Vec<u8>,
}

impl Presenter {
    pub fn new(wrap: Wrap, placing: Placing, cell: (u32, u32), transport: Transport) -> Presenter {
        Presenter {
            wrap,
            placing,
            cell: (cell.0.max(1), cell.1.max(1)),
            transport,
            size: (0, 0),
            base: None,
            patches: Vec::new(),
            overlay: None,
            next_id: 1,
            next_z: 1,
            owners: Vec::new(),
            counts: HashMap::new(),
            files: 0,
            scratch: Vec::new(),
            deflated: Vec::new(),
        }
    }

    pub fn placing(&self) -> Placing {
        self.placing
    }

    /// Starts over with a new cell size: the next frame is sent whole.
    pub fn set_cell(&mut self, cell: (u32, u32)) {
        self.cell = (cell.0.max(1), cell.1.max(1));
        self.base = None;
    }

    /// Forgets what the terminal shows, so the next frame is sent whole.
    pub fn invalidate(&mut self) {
        self.base = None;
    }

    /// Deletes every image this sent.
    pub fn clear(&mut self, out: &mut Vec<u8>) {
        let ids = self
            .base
            .into_iter()
            .chain(self.patches.iter().map(|p| p.id))
            .chain(self.overlay.map(|p| p.id))
            .chain(self.counts.keys().copied())
            .collect::<std::collections::BTreeSet<_>>();
        for id in ids {
            kitty::delete(out, self.wrap, id);
        }
        self.base = None;
        self.patches.clear();
        self.overlay = None;
        self.counts.clear();
        self.owners.clear();
    }

    /// Writes to `out` what shows `under`, and `overlay` (the whole frame
    /// with its overlays, and the pixels they cover) above it.
    pub fn present(
        &mut self,
        under: Frame<'_>,
        overlay: Option<(Frame<'_>, Rect)>,
        out: &mut Vec<u8>,
    ) -> Sent {
        let start = out.len();
        let mut sent = Sent::default();
        let (w, h) = under.size;
        if w == 0 || h == 0 {
            return sent;
        }
        let rects = if self.placing == Placing::Cells {
            dirty_rects(under, MAX_RECTS)
                .into_iter()
                .map(|r| snap(r, self.cell, under.size))
                .collect()
        } else {
            dirty_rects(under, MAX_RECTS)
        };
        let changed: u64 = rects.iter().map(|r| area(*r)).sum();
        let whole = self.base.is_none()
            || self.size != under.size
            || changed * 100 >= u64::from(w) * u64::from(h) * WHOLE_PERCENT
            || self.patches.len() + rects.len() > MAX_PATCHES
            || self.next_z >= OVERLAY_Z - 1;
        if whole {
            self.send_whole(under, out, &mut sent);
        } else {
            for r in rects {
                self.send_patch(under, r, out, &mut sent);
            }
        }
        if self.placing == Placing::Cursor {
            self.send_overlay(overlay, out, &mut sent);
        }
        sent.bytes = out.len() - start;
        sent.whole = whole;
        sent
    }

    fn send_whole(&mut self, f: Frame<'_>, out: &mut Vec<u8>, sent: &mut Sent) {
        let old = self.clear_under();
        self.size = f.size;
        let id = self.id();
        let full = [0, 0, f.size.0, f.size.1];
        match self.placing {
            Placing::Cursor => {
                out.extend_from_slice(b"\x1b[1;1H");
                self.send_image(
                    f,
                    full,
                    id,
                    Show::Cursor {
                        offset: (0, 0),
                        z: 0,
                    },
                    out,
                    sent,
                );
                self.next_z = 1;
            }
            Placing::Cells => {
                let (cols, rows) = self.cells_of(full);
                self.send_image(f, full, id, Show::Cells { cols, rows }, out, sent);
                kitty::placeholders(out, id, (0, 0), cols, rows);
                let n = (f.size.0 / self.cell.0) as usize * (f.size.1 / self.cell.1) as usize;
                self.owners.clear();
                self.owners.resize(n, id);
                self.counts.insert(id, n as u32);
            }
        }
        for id in old {
            kitty::delete(out, self.wrap, id);
        }
        self.base = Some(id);
    }

    /// Forgets the images under the overlay, answering their ids to delete
    /// once what replaces them is sent.
    fn clear_under(&mut self) -> Vec<u32> {
        let mut old: Vec<u32> = self.base.take().into_iter().collect();
        old.extend(self.patches.drain(..).map(|p| p.id));
        old.extend(self.counts.drain().map(|(id, _)| id));
        old.sort_unstable();
        old.dedup();
        old
    }

    fn send_patch(&mut self, f: Frame<'_>, r: Rect, out: &mut Vec<u8>, sent: &mut Sent) {
        let id = self.id();
        match self.placing {
            Placing::Cursor => {
                let (cw, ch) = self.cell;
                let _ = write!(out, "\x1b[{};{}H", r[1] / ch + 1, r[0] / cw + 1);
                let show = Show::Cursor {
                    offset: (r[0] % cw, r[1] % ch),
                    z: self.next_z,
                };
                self.next_z += 1;
                self.send_image(f, r, id, show, out, sent);
                // What the new patch covers whole is under it for good.
                let wrap = self.wrap;
                self.patches.retain(|p| {
                    let covered = contains(r, p.rect);
                    if covered {
                        kitty::delete(out, wrap, p.id);
                    }
                    !covered
                });
                self.patches.push(Patch { id, rect: r });
            }
            Placing::Cells => {
                let (cols, rows) = self.cells_of(r);
                if cols == 0 || rows == 0 {
                    return;
                }
                self.send_image(f, r, id, Show::Cells { cols, rows }, out, sent);
                let at = (r[0] / self.cell.0, r[1] / self.cell.1);
                kitty::placeholders(out, id, at, cols, rows);
                let grid_w = (self.size.0 / self.cell.0) as usize;
                self.counts.insert(id, cols * rows);
                for row in at.1..at.1 + rows {
                    for col in at.0..at.0 + cols {
                        let Some(owner) = self.owners.get_mut(row as usize * grid_w + col as usize)
                        else {
                            continue;
                        };
                        let was = std::mem::replace(owner, id);
                        if let Some(n) = self.counts.get_mut(&was) {
                            *n -= 1;
                            if *n == 0 {
                                self.counts.remove(&was);
                                kitty::delete(out, self.wrap, was);
                            }
                        }
                    }
                }
                self.patches.push(Patch { id, rect: r });
                self.patches.retain(|p| self.counts.contains_key(&p.id));
            }
        }
    }

    fn send_overlay(
        &mut self,
        overlay: Option<(Frame<'_>, Rect)>,
        out: &mut Vec<u8>,
        sent: &mut Sent,
    ) {
        let Some((f, r)) = overlay.filter(|(_, r)| area(*r) > 0) else {
            if let Some(old) = self.overlay.take() {
                kitty::delete(out, self.wrap, old.id);
            }
            return;
        };
        let same = self.overlay.is_some_and(|o| o.rect == r);
        if same && !touches_dirty(f, r) {
            return;
        }
        let id = self.id();
        let (cw, ch) = self.cell;
        let _ = write!(out, "\x1b[{};{}H", r[1] / ch + 1, r[0] / cw + 1);
        let show = Show::Cursor {
            offset: (r[0] % cw, r[1] % ch),
            z: OVERLAY_Z,
        };
        self.send_image(f, r, id, show, out, sent);
        if let Some(old) = self.overlay.replace(Patch { id, rect: r }) {
            kitty::delete(out, self.wrap, old.id);
        }
    }

    /// Sends rectangle `r` of `f` as image `id`.
    fn send_image(
        &mut self,
        f: Frame<'_>,
        r: Rect,
        id: u32,
        show: Show,
        out: &mut Vec<u8>,
        sent: &mut Sent,
    ) {
        let size = (r[2] - r[0], r[3] - r[1]);
        crop_rgb(f, r, &mut self.scratch);
        sent.images += 1;
        let file = self
            .transport
            .file_dir
            .as_ref()
            .filter(|_| self.scratch.len() >= FILE_MIN)
            .and_then(|dir| {
                self.files += 1;
                let path = dir.join(format!(
                    "tty-graphics-protocol-vorn-{}-{}",
                    std::process::id(),
                    self.files
                ));
                std::fs::write(&path, &self.scratch).ok().map(|()| path)
            });
        if let Some(path) = file {
            sent.file_bytes += self.scratch.len();
            let fmt = (size, Format::Rgb);
            kitty::transmit(out, self.wrap, id, fmt, show, Medium::File(&path), &[]);
            return;
        }
        let zlib = self.transport.zlib && self.scratch.len() >= ZLIB_MIN;
        let data = if zlib {
            deflate(&self.scratch, &mut self.deflated);
            &self.deflated
        } else {
            &self.scratch
        };
        let fmt = (size, Format::Rgb);
        kitty::transmit(out, self.wrap, id, fmt, show, Medium::Direct { zlib }, data);
    }

    fn cells_of(&self, r: Rect) -> (u32, u32) {
        (
            ((r[2] - r[0]) / self.cell.0).min(kitty::MAX_PLACEHOLDER_ROWS),
            ((r[3] - r[1]) / self.cell.1).min(kitty::MAX_PLACEHOLDER_ROWS),
        )
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = if id >= MAX_ID { 1 } else { id + 1 };
        id
    }
}

fn area(r: Rect) -> u64 {
    u64::from(r[2].saturating_sub(r[0])) * u64::from(r[3].saturating_sub(r[1]))
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

/// Whether a changed tile of `f` meets `r`.
fn touches_dirty(f: Frame<'_>, r: Rect) -> bool {
    let t = f.tile.max(1);
    let (tx0, ty0) = ((r[0] / t) as usize, (r[1] / t) as usize);
    let (tx1, ty1) = (
        (r[2].saturating_sub(1) / t) as usize,
        (r[3].saturating_sub(1) / t) as usize,
    );
    (ty0..=ty1).any(|ty| {
        (tx0..=tx1.min(f.tiles_x.saturating_sub(1)))
            .any(|tx| f.dirty.get(ty * f.tiles_x + tx).copied().unwrap_or(true))
    })
}

/// The changed tiles of `f` as rectangles in pixels, clipped to the frame:
/// runs along each row of tiles, joined down while their spans match. More
/// than `max` and their bounding box stands for them all.
pub fn dirty_rects(f: Frame<'_>, max: usize) -> Vec<Rect> {
    let tiles_x = f.tiles_x.max(1);
    let tiles_y = f.dirty.len() / tiles_x;
    let mut done: Vec<[usize; 4]> = Vec::new();
    let mut open: Vec<[usize; 4]> = Vec::new();
    for ty in 0..tiles_y {
        let row = &f.dirty[ty * tiles_x..(ty + 1) * tiles_x];
        let mut spans = Vec::new();
        let mut tx = 0;
        while tx < tiles_x {
            if !row[tx] {
                tx += 1;
                continue;
            }
            let x0 = tx;
            while tx < tiles_x && row[tx] {
                tx += 1;
            }
            spans.push((x0, tx));
        }
        let mut next = Vec::with_capacity(spans.len());
        for (x0, x1) in spans {
            match open.iter().position(|r| r[0] == x0 && r[2] == x1) {
                Some(i) => {
                    let mut r = open.swap_remove(i);
                    r[3] = ty + 1;
                    next.push(r);
                }
                None => next.push([x0, ty, x1, ty + 1]),
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    if done.len() > max {
        let b = done.iter().fold([usize::MAX, usize::MAX, 0, 0], |a, r| {
            [
                a[0].min(r[0]),
                a[1].min(r[1]),
                a[2].max(r[2]),
                a[3].max(r[3]),
            ]
        });
        done = vec![b];
    }
    let t = f.tile as usize;
    let px = |v: usize, max: u32| (v * t).min(max as usize) as u32;
    done.into_iter()
        .map(|r| {
            [
                px(r[0], f.size.0),
                px(r[1], f.size.1),
                px(r[2], f.size.0),
                px(r[3], f.size.1),
            ]
        })
        .filter(|r| area(*r) > 0)
        .collect()
}

/// `r` grown to whole cells, within the cells the frame holds.
fn snap(r: Rect, (cw, ch): (u32, u32), size: (u32, u32)) -> Rect {
    let (gw, gh) = (size.0 / cw * cw, size.1 / ch * ch);
    [
        (r[0] / cw * cw).min(gw),
        (r[1] / ch * ch).min(gh),
        r[2].div_ceil(cw).saturating_mul(cw).min(gw),
        r[3].div_ceil(ch).saturating_mul(ch).min(gh),
    ]
}

/// Rectangle `r` of `f` as packed RGB rows into `out`.
fn crop_rgb(f: Frame<'_>, r: Rect, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(area(r) as usize * 3);
    let w = f.size.0 as usize;
    for y in r[1] as usize..r[3] as usize {
        let row = &f.px[y * w + r[0] as usize..y * w + r[2] as usize];
        for p in row {
            let [a, b, c, _] = p.to_ne_bytes();
            out.extend_from_slice(&[a, b, c]);
        }
    }
}

/// `data` deflated into `out`, fast over small: frames are mostly flat
/// fills, which the fastest level already shrinks many times.
fn deflate(data: &[u8], out: &mut Vec<u8>) {
    use flate2::{write::ZlibEncoder, Compression};
    out.clear();
    let mut z = ZlibEncoder::new(std::mem::take(out), Compression::fast());
    let _ = z.write_all(data);
    *out = z.finish().unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u32 = 4;

    /// A `w` x `h` frame of one colour with tiles of `T` pixels.
    struct Owned {
        px: Vec<u32>,
        size: (u32, u32),
        dirty: Vec<bool>,
    }

    impl Owned {
        fn new(w: u32, h: u32, rgba: [u8; 4]) -> Owned {
            let tiles = (w.div_ceil(T) * h.div_ceil(T)) as usize;
            Owned {
                px: vec![u32::from_ne_bytes(rgba); (w * h) as usize],
                size: (w, h),
                dirty: vec![true; tiles],
            }
        }

        fn frame(&self) -> Frame<'_> {
            Frame {
                px: &self.px,
                size: self.size,
                dirty: &self.dirty,
                tiles_x: self.size.0.div_ceil(T) as usize,
                tile: T,
            }
        }

        fn clean(&mut self) {
            self.dirty.iter_mut().for_each(|d| *d = false);
        }

        fn touch(&mut self, tx: usize, ty: usize) {
            let tiles_x = self.size.0.div_ceil(T) as usize;
            self.dirty[ty * tiles_x + tx] = true;
        }
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    #[test]
    fn rects_join_runs_down_while_their_spans_match() {
        let mut f = Owned::new(16, 16, [0; 4]);
        f.clean();
        f.touch(1, 0);
        f.touch(2, 0);
        f.touch(1, 1);
        f.touch(2, 1);
        f.touch(0, 3);
        assert_eq!(
            dirty_rects(f.frame(), 8),
            vec![[4, 0, 12, 8], [0, 12, 4, 16]]
        );
        assert_eq!(
            dirty_rects(f.frame(), 1),
            vec![[0, 0, 12, 16]],
            "too many: their bounding box"
        );
    }

    #[test]
    fn rects_stop_at_the_frame_edge() {
        let f = Owned::new(6, 5, [0; 4]);
        assert_eq!(dirty_rects(f.frame(), 8), vec![[0, 0, 6, 5]]);
    }

    #[test]
    fn the_first_frame_is_one_image_at_the_top_left() {
        let f = Owned::new(2, 1, [10, 20, 30, 255]);
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (1, 1), Transport::default());
        let mut out = Vec::new();
        let sent = p.present(f.frame(), None, &mut out);
        assert_eq!(
            text(&out),
            "\x1b[1;1H\x1b_Ga=T,f=24,s=2,v=1,i=1,q=2,C=1,z=0,m=0;ChQeChQe\x1b\\"
        );
        assert!(sent.whole);
        assert_eq!((sent.images, sent.bytes), (1, out.len()));
    }

    #[test]
    fn a_change_sends_only_its_tiles_placed_over_the_frame() {
        let mut f = Owned::new(16, 16, [0; 4]);
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (3, 5), Transport::default());
        let mut out = Vec::new();
        p.present(f.frame(), None, &mut out);
        f.clean();
        f.touch(1, 1);
        out.clear();
        let sent = p.present(f.frame(), None, &mut out);
        assert!(!sent.whole);
        // Tile (1, 1) is pixels 4..8: cell (1, 0) and 1 and 4 pixels in.
        let s = text(&out);
        assert!(
            s.starts_with("\x1b[1;2H\x1b_Ga=T,f=24,s=4,v=4,i=2,q=2,C=1,z=1,X=1,Y=4,m=0;"),
            "{s:?}"
        );
        out.clear();
        let sent = p.present(f.frame(), None, &mut out);
        assert!(
            text(&out).ends_with("\x1b_Ga=d,d=I,i=2,q=2\x1b\\"),
            "the patch it covers goes"
        );
        assert_eq!(sent.images, 1);
        f.clean();
        out.clear();
        let sent = p.present(f.frame(), None, &mut out);
        assert_eq!(
            (sent.images, out.len()),
            (0, 0),
            "nothing changed, nothing sent"
        );
    }

    #[test]
    fn a_large_change_resends_the_frame_and_drops_the_patches() {
        let mut f = Owned::new(16, 16, [0; 4]);
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (4, 4), Transport::default());
        let mut out = Vec::new();
        p.present(f.frame(), None, &mut out);
        f.clean();
        f.touch(0, 0);
        p.present(f.frame(), None, &mut out);
        f.dirty.iter_mut().take(12).for_each(|d| *d = true);
        out.clear();
        let sent = p.present(f.frame(), None, &mut out);
        assert!(sent.whole);
        let s = text(&out);
        assert!(s.contains("i=3,q=2,C=1,z=0"));
        assert!(s.ends_with("\x1b_Ga=d,d=I,i=1,q=2\x1b\\\x1b_Ga=d,d=I,i=2,q=2\x1b\\"));
    }

    #[test]
    fn an_overlay_is_its_own_image_on_top_and_closing_it_deletes_it() {
        let mut f = Owned::new(16, 16, [0; 4]);
        let all = Owned::new(16, 16, [9; 4]);
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (4, 4), Transport::default());
        let mut out = Vec::new();
        p.present(f.frame(), None, &mut out);
        f.clean();
        out.clear();
        let sent = p.present(f.frame(), Some((all.frame(), [5, 6, 9, 8])), &mut out);
        assert_eq!(sent.images, 1);
        assert!(text(&out)
            .starts_with("\x1b[2;2H\x1b_Ga=T,f=24,s=4,v=2,i=2,q=2,C=1,z=1073741824,X=1,Y=2,m=0;"));
        out.clear();
        p.present(f.frame(), None, &mut out);
        assert_eq!(text(&out), "\x1b_Ga=d,d=I,i=2,q=2\x1b\\");
    }

    #[test]
    fn inside_tmux_changes_replace_whole_placeholder_cells() {
        let mut f = Owned::new(16, 8, [0; 4]);
        let mut p = Presenter::new(Wrap::Tmux, Placing::Cells, (8, 4), Transport::default());
        let mut out = Vec::new();
        let sent = p.present(f.frame(), None, &mut out);
        assert!(sent.whole);
        let s = text(&out);
        assert!(s.starts_with("\x1bPtmux;\x1b\x1b_Ga=T,f=24,s=16,v=8,i=1,q=2,U=1,c=2,r=2,m=0;"));
        assert!(s.contains("\x1b[38;2;0;0;1m\x1b[1;1H\u{10EEEE}"));
        f.clean();
        f.touch(2, 0);
        f.touch(3, 0);
        out.clear();
        p.present(f.frame(), None, &mut out);
        let s = text(&out);
        assert!(s.contains("s=8,v=4,i=2,q=2,U=1,c=1,r=1"), "{s:?}");
        assert!(s.contains("\x1b[1;2H\u{10EEEE}"));
        assert!(!s.contains("d=I"), "the frame still shows in three cells");
        f.clean();
        f.touch(2, 0);
        out.clear();
        p.present(f.frame(), None, &mut out);
        assert!(
            text(&out).ends_with("\x1b_Ga=d,d=I,i=2,q=2\x1b\x1b\\\x1b\\"),
            "a patch with no cells left goes"
        );
    }

    #[test]
    fn big_images_go_through_a_file_and_inline_ones_deflate() {
        let dir = std::env::temp_dir().join(format!("vornui-present-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let f = Owned::new(128, 128, [1, 2, 3, 255]);
        let transport = Transport {
            file_dir: Some(dir.clone()),
            zlib: false,
        };
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (8, 16), transport);
        let mut out = Vec::new();
        let sent = p.present(f.frame(), None, &mut out);
        assert_eq!(sent.file_bytes, 128 * 128 * 3);
        assert!(text(&out).contains(",t=t;"));
        let transport = Transport {
            file_dir: None,
            zlib: true,
        };
        let mut p = Presenter::new(Wrap::Direct, Placing::Cursor, (8, 16), transport);
        out.clear();
        let sent = p.present(f.frame(), None, &mut out);
        assert!(text(&out).contains(",o=z,m=0;"));
        assert!(
            sent.bytes < 2048,
            "a flat frame deflates to little: {}",
            sent.bytes
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
