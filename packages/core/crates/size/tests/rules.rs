//! The size rule's acceptance tests (Terminal State Protocol §16, T9 to
//! T9e), on the policy alone with an injected clock: a desktop, a phone and
//! a browser doing what people do, minutes of it in microseconds.
//!
//! The host here polls exactly when the policy says it next has something to
//! do, as vornd's driver does, and counts every resize it would send.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use vorn_size::{fit, typed, Decision, Event, Policy, Presence, Reason, Size};

const DESKTOP: u8 = 1;
const PHONE: u8 = 2;
const WEB: u8 = 3;

/// The renderer's font and its floor (TP §10: 9 px).
const FONT: f32 = 13.0;
const MIN_FONT: f32 = 9.0;

/// A session, its clients and a host that applies every resize at once.
struct Sim {
    p: Policy<u8>,
    t0: Instant,
    now: Instant,
    resizes: Vec<Decision<u8>>,
    /// Each client's viewport, for the fit check.
    views: BTreeMap<u8, Size>,
    /// Fit checks made, so a test can say it made some.
    fits: usize,
}

impl Sim {
    fn new(launch: Size) -> Sim {
        let t0 = Instant::now();
        Sim {
            p: Policy::new(launch),
            t0,
            now: t0,
            resizes: Vec::new(),
            views: BTreeMap::new(),
            fits: 0,
        }
    }

    fn ms(&self) -> u128 {
        self.now.duration_since(self.t0).as_millis()
    }

    /// Moves the clock on by `ms`, polling whenever the policy is due on the
    /// way, as the host's timer would.
    fn wait(&mut self, ms: u64) {
        let until = self.now + Duration::from_millis(ms);
        while let Some(due) = self.p.due().filter(|&d| d <= until) {
            self.now = self.now.max(due);
            self.poll();
        }
        self.now = until;
        self.poll();
    }

    fn poll(&mut self) {
        if let Some(d) = self.p.poll(self.now) {
            // The record comes straight back.
            self.p.applied(d.size);
            self.resizes.push(d);
        }
        self.check_fit();
    }

    fn on(&mut self, e: Event<u8>) {
        if let Event::Viewport { client, size } = e {
            self.views.insert(client, size);
        }
        if let Event::Attach {
            client,
            viewport: Some(v),
            ..
        } = e
        {
            self.views.insert(client, v);
        }
        if let Event::Detach { client } = e {
            self.views.remove(&client);
        }
        self.p.on(e, self.now);
        self.poll();
    }

    fn attach(&mut self, client: u8, desktop: bool, cols: u16, rows: u16) {
        self.on(Event::Attach {
            client,
            desktop,
            opener: false,
            viewport: Some(Size::new(cols, rows)),
            presence: Presence::Watching,
        });
    }

    fn view(&mut self, client: u8, cols: u16, rows: u16) {
        self.on(Event::Viewport {
            client,
            size: Size::new(cols, rows),
        });
    }

    fn presence(&mut self, client: u8, state: Presence) {
        self.on(Event::Presence { client, state });
    }

    /// What a bytes client writes: input only if a person did something.
    fn write(&mut self, client: u8, bytes: &[u8]) {
        if typed(bytes) {
            self.on(Event::Input { client });
        }
    }

    fn type_key(&mut self, client: u8) {
        self.write(client, b"x");
    }

    /// T9e's invariant, after every step: every client draws every column
    /// and row of the session, on screen or a pan away, and one whose box
    /// holds the grid pans nothing.
    fn check_fit(&mut self) {
        let grid = self.p.size();
        for (&c, &v) in &self.views {
            let f = fit(grid, v, FONT, MIN_FONT);
            assert_eq!(
                (f.shown.cols + f.pan.cols, f.shown.rows + f.pan.rows),
                (grid.cols, grid.rows),
                "client {c} at {v:?} clips {grid:?}: {f:?}"
            );
            assert!(f.font >= MIN_FONT && f.font <= FONT, "{f:?}");
            if v.cols >= grid.cols && v.rows >= grid.rows {
                assert_eq!(f.pan, Size::default(), "client {c} pans a grid it holds");
                assert_eq!(f.font, FONT);
            }
            self.fits += 1;
        }
    }

    fn sizes(&self) -> Vec<(u16, u16)> {
        self.resizes
            .iter()
            .map(|d| (d.size.cols, d.size.rows))
            .collect()
    }
}

/// A desktop at 120x40 that has typed into the session and owns its size.
fn desktop_owns() -> Sim {
    let mut s = Sim::new(Size::new(120, 40));
    s.attach(DESKTOP, true, 120, 40);
    s.type_key(DESKTOP);
    s.wait(1000);
    assert_eq!(s.p.owner(), Some(DESKTOP));
    assert!(s.resizes.is_empty());
    s
}

/// TP-T9: looking doesn't resize. A desktop and a phone at different sizes;
/// the phone attaches, focuses, scrolls and backgrounds 50 times with no
/// typing: zero resizes. Focus and wheel reports are not typing.
#[test]
fn t9_looking_does_not_resize() {
    let mut s = desktop_owns();
    // Long enough that the desktop is quiet: a takeover would be allowed.
    s.wait(60_000);
    for i in 0..50u16 {
        s.attach(PHONE, false, 50 + i % 3, 30);
        s.presence(PHONE, Presence::Watching);
        s.write(PHONE, b"\x1b[I");
        s.presence(PHONE, Presence::Active);
        s.write(PHONE, b"\x1b[<65;10;10M\x1b[<64;10;10M");
        s.view(PHONE, 30, 50);
        s.wait(800);
        s.presence(PHONE, Presence::Away);
        s.write(PHONE, b"\x1b[O");
        s.wait(15_000);
        s.on(Event::Detach { client: PHONE });
        s.wait(1_000);
    }
    assert_eq!(s.resizes, Vec::new(), "looking resized");
    assert_eq!(s.p.size(), Size::new(120, 40));
    assert_eq!(s.p.owner(), Some(DESKTOP));
}

/// TP-T9a: typing on the phone after 3 s of desktop silence is one resize
/// to the phone's viewport; within 3 s it is none. Locking the phone gives
/// the desktop its size back after the grace period, in one resize.
#[test]
fn t9a_typing_takes_the_size_and_it_comes_back() {
    let mut s = desktop_owns();
    s.attach(PHONE, false, 50, 30);
    // The desktop typed a moment ago: the phone's input does not take it.
    s.type_key(DESKTOP);
    s.wait(1_000);
    s.type_key(PHONE);
    s.wait(1_500);
    assert!(s.resizes.is_empty(), "{:?}", s.resizes);
    // Three seconds of desktop silence, and the phone types.
    s.wait(1_000);
    s.type_key(PHONE);
    for _ in 0..20 {
        s.wait(300);
        s.type_key(PHONE);
    }
    assert_eq!(s.sizes(), [(50, 30)]);
    assert_eq!(s.resizes[0].owner, Some(PHONE));
    assert_eq!(s.resizes[0].reason, Reason::Input);
    // The phone is locked; the desktop is on screen.
    s.presence(PHONE, Presence::Away);
    let away = s.ms();
    s.wait(9_000);
    assert_eq!(s.resizes.len(), 1, "not before the grace period");
    s.wait(2_000);
    assert_eq!(s.sizes(), [(50, 30), (120, 40)]);
    let back = s.resizes[1];
    assert_eq!((back.owner, back.reason), (Some(DESKTOP), Reason::Returned));
    assert!(s.ms() - away >= 10_000);
    // A phone locked for a moment costs nothing.
    s.wait(60_000);
    s.type_key(PHONE);
    s.wait(500);
    s.presence(PHONE, Presence::Away);
    s.wait(4_000);
    s.presence(PHONE, Presence::Active);
    s.type_key(PHONE);
    s.wait(30_000);
    assert_eq!(s.sizes(), [(50, 30), (120, 40), (50, 30)]);
}

/// TP-T9a, the last clause, on the policy's side: whether a client is a
/// desktop is the host's to say. A browser the host calls remote loses to
/// the desktop when both type, whatever it calls itself.
#[test]
fn t9a_a_remote_client_does_not_win_ties() {
    let mut s = Sim::new(Size::new(100, 30));
    s.attach(WEB, false, 90, 25);
    s.attach(DESKTOP, true, 120, 40);
    s.type_key(WEB);
    s.wait(400);
    s.type_key(DESKTOP);
    s.wait(400);
    s.type_key(WEB);
    s.wait(1_000);
    assert_eq!(s.p.owner(), Some(DESKTOP));
    assert_eq!(s.sizes(), [(90, 25), (120, 40)]);
}

/// TP-T9b: no ping-pong. Two clients typing alternately every 500 ms for a
/// minute: at most one resize, and the desktop keeps the size.
#[test]
fn t9b_no_ping_pong() {
    let mut s = Sim::new(Size::new(100, 30));
    s.attach(DESKTOP, true, 120, 40);
    s.attach(PHONE, false, 50, 30);
    for i in 0..120 {
        s.type_key(if i % 2 == 0 { DESKTOP } else { PHONE });
        s.wait(500);
    }
    s.wait(30_000);
    assert!(s.resizes.len() <= 1, "{:?}", s.resizes);
    assert_eq!(s.p.owner(), Some(DESKTOP));
    assert_eq!(s.p.size(), Size::new(120, 40));
}

/// TP-T9c: fewer SIGWINCHes. A window drag, a phone rotation and a
/// 1-column snap produce at most one, one and zero resizes.
#[test]
fn t9c_drags_rotations_and_snaps() {
    // A drag: a new box every frame for a second.
    let mut s = desktop_owns();
    for i in 0..60u16 {
        s.view(DESKTOP, 120 + i, 40 + i / 6);
        s.wait(16);
    }
    s.wait(1_000);
    assert_eq!(s.sizes(), [(179, 49)]);

    // A rotation, with the keyboard sliding up and down on the way.
    let mut s = Sim::new(Size::new(120, 40));
    s.attach(PHONE, false, 50, 30);
    s.type_key(PHONE);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(50, 30)]);
    for (cols, rows, ms) in [(70, 24, 30), (90, 18, 40), (90, 11, 60), (90, 18, 50)] {
        s.view(PHONE, cols, rows);
        s.wait(ms);
    }
    s.wait(1_000);
    assert_eq!(s.sizes(), [(50, 30), (90, 18)]);

    // A snap by a column, by two, and by a row: near misses all.
    let mut s = desktop_owns();
    for (cols, rows) in [(121, 40), (122, 40), (118, 41), (120, 39), (119, 40)] {
        s.view(DESKTOP, cols, rows);
        s.wait(1_000);
    }
    assert_eq!(s.resizes, Vec::new());
    // Three columns is a resize.
    s.view(DESKTOP, 123, 40);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(123, 40)]);
}

/// A tiny seeded generator, so the trace is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One step of an activity trace.
#[derive(Debug, Clone, Copy)]
enum Step {
    Wait(u64),
    Ev(Event<u8>),
}

/// An hour of a desktop and a phone on one agent session, the activity the
/// rule was written against: the desktop in use in bursts with window drags
/// and snaps between them; the phone glanced at every few minutes (unlocked,
/// scrolled, locked again), now and then rotated, and sometimes typed into.
fn hour_of_use(seed: u64) -> Vec<Step> {
    let mut r = Rng(seed);
    let mut t = Vec::new();
    let ev = |e| Step::Ev(e);
    t.push(ev(Event::Attach {
        client: DESKTOP,
        desktop: true,
        opener: false,
        viewport: Some(Size::new(120, 40)),
        presence: Presence::Watching,
    }));
    t.push(ev(Event::Attach {
        client: PHONE,
        desktop: false,
        opener: false,
        viewport: Some(Size::new(50, 30)),
        presence: Presence::Away,
    }));
    let mut desk = Size::new(120, 40);
    let mut elapsed = 0u64;
    while elapsed < 3_600_000 {
        match r.below(10) {
            // Desktop work: a burst of keys.
            0..=4 => {
                for _ in 0..(5 + r.below(40)) {
                    t.push(ev(Event::Input { client: DESKTOP }));
                    t.push(Step::Wait(150 + r.below(400)));
                }
            }
            // The window: a drag, or a snap of a column or two.
            5 => {
                if r.below(2) == 0 {
                    let to = Size::new(100 + r.below(60) as u16, 30 + r.below(15) as u16);
                    for i in 0..20u16 {
                        let mid = Size::new(
                            desk.cols + (to.cols.abs_diff(desk.cols) * i / 20),
                            desk.rows,
                        );
                        t.push(ev(Event::Viewport {
                            client: DESKTOP,
                            size: mid,
                        }));
                        t.push(Step::Wait(16));
                    }
                    desk = to;
                } else {
                    desk = Size::new(desk.cols + 1, desk.rows);
                }
                t.push(ev(Event::Viewport {
                    client: DESKTOP,
                    size: desk,
                }));
            }
            // A glance at the phone: unlock, scroll, maybe rotate, lock.
            6..=8 => {
                t.push(ev(Event::Presence {
                    client: PHONE,
                    state: Presence::Watching,
                }));
                t.push(ev(Event::Presence {
                    client: PHONE,
                    state: Presence::Active,
                }));
                if r.below(4) == 0 {
                    t.push(ev(Event::Viewport {
                        client: PHONE,
                        size: Size::new(90, 18),
                    }));
                    t.push(Step::Wait(2_000));
                    t.push(ev(Event::Viewport {
                        client: PHONE,
                        size: Size::new(50, 30),
                    }));
                }
                t.push(Step::Wait(3_000 + r.below(20_000)));
                t.push(ev(Event::Presence {
                    client: PHONE,
                    state: Presence::Away,
                }));
            }
            // Away from the desk, typing on the phone.
            _ => {
                t.push(ev(Event::Presence {
                    client: DESKTOP,
                    state: Presence::Away,
                }));
                t.push(Step::Wait(5_000));
                t.push(ev(Event::Presence {
                    client: PHONE,
                    state: Presence::Active,
                }));
                for _ in 0..(3 + r.below(20)) {
                    t.push(ev(Event::Input { client: PHONE }));
                    t.push(Step::Wait(300 + r.below(800)));
                }
                t.push(ev(Event::Presence {
                    client: PHONE,
                    state: Presence::Away,
                }));
                t.push(ev(Event::Presence {
                    client: DESKTOP,
                    state: Presence::Watching,
                }));
            }
        }
        let pause = 5_000 + r.below(60_000);
        t.push(Step::Wait(pause));
        elapsed = t
            .iter()
            .map(|s| match s {
                Step::Wait(ms) => *ms,
                Step::Ev(_) => 0,
            })
            .sum();
    }
    t
}

/// tmux's `window-size latest` replayed on the same trace: the size of the
/// client used most recently, where using is any activity in the pane
/// (a key, a focus, a scroll) and every size change of that client counts.
fn tmux_latest(trace: &[Step]) -> usize {
    let mut views: BTreeMap<u8, Size> = BTreeMap::new();
    let mut size: Option<Size> = None;
    let mut latest: Option<u8> = None;
    let mut resizes = 0;
    for step in trace {
        let Step::Ev(e) = *step else { continue };
        let used = match e {
            Event::Attach {
                client, viewport, ..
            } => {
                if let Some(v) = viewport {
                    views.insert(client, v);
                }
                Some(client)
            }
            Event::Viewport { client, size } => {
                views.insert(client, size);
                (latest == Some(client)).then_some(client)
            }
            Event::Presence {
                client,
                state: Presence::Active | Presence::Watching,
            } => Some(client),
            Event::Input { client } => Some(client),
            _ => None,
        };
        if let Some(c) = used {
            latest = Some(c);
            let want = views.get(&c).copied();
            if want.is_some() && want != size {
                if size.is_some() {
                    resizes += 1;
                }
                size = want;
            }
        }
    }
    resizes
}

/// TP-T9c, the comparison: against tmux-style `latest` replayed on the same
/// activity, the resizes per hour drop, and the counts are reported.
#[test]
fn t9c_fewer_resizes_per_hour_than_tmux_latest() {
    let mut total = (0, 0);
    for seed in [7, 0x5eed, 0xdecaf, 42] {
        let trace = hour_of_use(seed);
        let mut s = Sim::new(Size::new(120, 40));
        for step in &trace {
            match *step {
                Step::Wait(ms) => s.wait(ms),
                Step::Ev(e) => s.on(e),
            }
        }
        let ours = s.resizes.len();
        let theirs = tmux_latest(&trace);
        println!("seed {seed:#x}: {ours} resizes in an hour against {theirs} for tmux latest");
        assert!(ours < theirs, "seed {seed:#x}: {ours} against {theirs}");
        total.0 += ours;
        total.1 += theirs;
    }
    println!("all seeds: {} against {}", total.0, total.1);
}

/// TP-T9d: quiet agents keep their size. A workflow-launched agent stays at
/// its launch size while clients of three sizes attach and watch, until one
/// of them types.
#[test]
fn t9d_quiet_agents_keep_their_size() {
    let mut s = Sim::new(Size::new(100, 30));
    s.attach(DESKTOP, true, 120, 40);
    s.attach(PHONE, false, 50, 30);
    s.attach(WEB, false, 90, 25);
    for i in 0..30u16 {
        for c in [DESKTOP, PHONE, WEB] {
            s.presence(c, Presence::Active);
            s.write(c, b"\x1b[I\x1b[<64;1;1M");
            s.view(c, 60 + i * 2, 20 + i % 5);
            s.wait(2_000);
            s.presence(
                c,
                if i % 3 == 0 {
                    Presence::Away
                } else {
                    Presence::Watching
                },
            );
        }
        s.wait(10_000);
    }
    assert_eq!(s.resizes, Vec::new());
    assert_eq!(s.p.size(), Size::new(100, 30));
    assert!(!s.p.typed());
    assert_eq!(s.p.owner(), None);
    // Someone types: from here the rule applies.
    s.view(WEB, 90, 25);
    s.write(WEB, b"y\r");
    s.wait(1_000);
    assert_eq!(s.sizes(), [(90, 25)]);
    assert!(s.p.typed());
}

/// TP-T9e: viewers fit. In every state the scenarios above pass through
/// (each one checks after every step), and at extremes, no client clips:
/// each draws all of the session's columns and rows, scaled or panned.
#[test]
fn t9e_viewers_fit_in_every_state() {
    let mut s = Sim::new(Size::new(300, 80));
    s.attach(DESKTOP, true, 120, 40);
    s.attach(PHONE, false, 1, 1);
    s.attach(WEB, false, 400, 100);
    for (who, cols, rows) in [
        (PHONE, 40, 60),
        (WEB, 10, 300),
        (DESKTOP, 2, 2),
        (PHONE, 65535, 65535),
    ] {
        s.view(who, cols, rows);
        s.type_key(who);
        s.wait(5_000);
    }
    assert!(s.fits > 0);
    // And through the other scenarios, which make the same check at every
    // step of their own.
    for scenario in [
        t9_looking_does_not_resize as fn(),
        t9a_typing_takes_the_size_and_it_comes_back,
        t9b_no_ping_pong,
        t9c_drags_rotations_and_snaps,
        t9d_quiet_agents_keep_their_size,
    ] {
        scenario();
    }
}

impl Sim {
    /// The client that asked for the session attaches, as the desktop's pane
    /// does: no viewport at first, then what fits in its box.
    fn open(&mut self, client: u8, desktop: bool, cols: u16, rows: u16) {
        self.on(Event::Attach {
            client,
            desktop,
            opener: true,
            viewport: None,
            presence: Presence::Watching,
        });
        self.view(client, cols, rows);
    }
}

/// A new terminal fits its pane on first draw: launched at the default
/// 80x24, it takes the viewport of the pane that opened it once, before
/// anyone types, and follows that pane's box from there. Others looking at
/// it still resize nothing, and it never goes home to them.
#[test]
fn a_new_terminal_fits_its_pane_on_first_draw() {
    let mut s = Sim::new(Size::new(80, 24));
    s.open(DESKTOP, true, 200, 50);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(200, 50)]);
    assert_eq!(
        (s.resizes[0].owner, s.resizes[0].reason),
        (Some(DESKTOP), Reason::Launch)
    );
    assert!(!s.p.typed());

    // A phone looks: nothing.
    s.attach(PHONE, false, 50, 30);
    s.presence(PHONE, Presence::Active);
    s.write(PHONE, b"\x1b[I\x1b[<64;1;1M");
    s.wait(5_000);
    // The pane's window is dragged wider: the session follows it.
    s.view(DESKTOP, 220, 50);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(200, 50), (220, 50)]);

    // The pane hides, then closes: the size stays where it is.
    s.presence(DESKTOP, Presence::Away);
    s.wait(30_000);
    s.on(Event::Detach { client: DESKTOP });
    s.wait(30_000);
    assert_eq!(s.sizes(), [(200, 50), (220, 50)]);

    // Typing is what takes it from here.
    s.type_key(PHONE);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(200, 50), (220, 50), (50, 30)]);
    assert_eq!(s.resizes[2].reason, Reason::Input);
}

/// The opener's first fit waits for its pane to be on screen, and its own
/// typing makes it an ordinary owner.
#[test]
fn the_opener_fits_once_shown_and_owns_by_input_once_it_types() {
    let mut s = Sim::new(Size::new(80, 24));
    s.on(Event::Attach {
        client: DESKTOP,
        desktop: true,
        opener: true,
        viewport: Some(Size::new(150, 45)),
        presence: Presence::Away,
    });
    s.views.insert(DESKTOP, Size::new(150, 45));
    s.wait(5_000);
    assert!(s.resizes.is_empty(), "a hidden pane has no box to fit");
    s.presence(DESKTOP, Presence::Watching);
    s.wait(1_000);
    assert_eq!(s.sizes(), [(150, 45)]);
    s.type_key(DESKTOP);
    s.wait(1_000);
    s.view(DESKTOP, 160, 45);
    s.wait(1_000);
    assert_eq!(s.resizes[1].reason, Reason::Input);
    // Owned by input now: away past the grace period, it goes home to a watcher.
    s.attach(WEB, true, 100, 30);
    s.presence(DESKTOP, Presence::Away);
    s.wait(11_000);
    assert_eq!(s.sizes(), [(150, 45), (160, 45), (100, 30)]);
}

/// The opener takes nothing from a session someone has typed into, nor from
/// a lock another client holds.
#[test]
fn the_opener_never_takes_an_established_or_locked_size() {
    let mut s = Sim::new(Size::new(80, 24));
    s.attach(PHONE, false, 50, 30);
    s.on(Event::LockSize {
        client: PHONE,
        locked: true,
    });
    s.wait(1_000);
    s.open(DESKTOP, true, 200, 50);
    s.wait(5_000);
    assert_eq!(s.sizes(), [(50, 30)]);
    assert_eq!(s.p.owner(), Some(PHONE));

    let mut s = Sim::new(Size::new(80, 24));
    s.attach(WEB, false, 90, 25);
    s.type_key(WEB);
    s.wait(1_000);
    s.open(DESKTOP, true, 200, 50);
    s.wait(30_000);
    assert_eq!(s.sizes(), [(90, 25)]);
    assert_eq!(s.p.owner(), Some(WEB));
}
