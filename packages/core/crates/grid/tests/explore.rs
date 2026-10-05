use libghostty_vt::terminal::{Options, Point, PointCoordinate, PointSpace, Terminal};
use std::time::Instant;
#[test]
fn explore() {
    for sb in [2usize << 20, 64 << 20] {
        let mut t = Terminal::new(Options {
            cols: 80,
            rows: 24,
            max_scrollback: sb,
        })
        .unwrap();
        let mut s = String::new();
        for n in 0..400_000 {
            s.push_str(&format!("{n}\r\n"));
        }
        let t0 = Instant::now();
        t.vt_write(s.as_bytes());
        let parse = t0.elapsed();
        let a = t
            .track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
            .unwrap();
        t.vt_write(b"x\r\ny\r\n");
        let t0 = Instant::now();
        let mut h = 0;
        for _ in 0..100 {
            h = t.scrollback_rows().unwrap();
        }
        let sbr = t0.elapsed() / 100;
        let t0 = Instant::now();
        let mut p = None;
        for _ in 0..100 {
            p = a.point(PointSpace::Screen).unwrap();
        }
        let pt = t0.elapsed() / 100;
        let t0 = Instant::now();
        for _ in 0..100 {
            let _ = a.point(PointSpace::Active).unwrap();
        }
        let pa = t0.elapsed() / 100;
        let t0 = Instant::now();
        for _ in 0..100 {
            let _ = t.total_rows().unwrap();
        }
        let tr = t0.elapsed() / 100;
        let t0 = Instant::now();
        for _ in 0..100 {
            let _ = t
                .track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
                .unwrap();
        }
        let tg = t0.elapsed() / 100;
        println!("sb {sb}: parse {parse:?} H {h} scrollback_rows {sbr:?} point(Screen) {pt:?} {p:?} point(Active) {pa:?} total_rows {tr:?} track {tg:?}");
    }
}
