# vorn-recovery

The test harness for session recovery. It plays sessiond (the record log, the
checkpoint store), feeds a session engine records, kills it at chosen, random
or timed points, recovers it, and compares its terminal with one that never
died.

- `gen`: seeded VT. Shell output, full-screen redraws, resizes, OSC titles and
  cwds, hyperlinks, kitty keyboard flags, queries, and UTF-8 and escape
  sequences split across records, cut off or aborted. The same seed gives the
  same log on every platform.
- `transcript`: real programs recorded from a PTY (vim, htop, an agent CLI).
- `compare`: the state equivalence. Two terminals are equal when the
  dimensions, screen and scrollback (as formatter VT), cursor, modes, kitty
  keyboard stacks, active screen, inactive screen and saved cursors,
  scrolling region, title and cwd all match. A mismatch lists every failing
  check with a readable diff.
- `driver`: the kill driver and the differential test.
- `child`: the same with the engine in a child process killed by the OS.
- `scan`: whether a record boundary is a safe checkpoint point (parser in
  ground, no UTF-8 sequence open).

## The differential test in one call

```rust
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{differential, InProcess, KillPlan, ReferenceConfig, ReferenceEngine, Restore};

let log = Generator::log(7, Profile::round_trip().bytes(1 << 20));
let report = differential(&log, &KillPlan::random(7, 5), || {
    Ok(InProcess::<ReferenceEngine>::new(ReferenceConfig::default(), Restore::Checkpoint))
})?;
assert_eq!(report.digest, log.digest()); // every byte, once
```

It runs the log through one target that is never killed and one killed after
five random records, recovers the killed one each time, and fails with
`Error::Mismatch` if their terminals differ, or `Error::Stream` if the
recovered run was not built from the log's bytes exactly once.

Other plans: `KillPlan::at([10, 500])` kills after those rseqs, and
`KillPlan::chaos(seed, every, jitter, over)` kills on a timer while the log is
delivered over `over`. For real process kills, make the target
`ChildProcess::new(env!("CARGO_BIN_EXE_recovery-subject"), config, restore)`.

## Plugging in a real engine

`ReferenceEngine` is a test double: a `vorn_screen::Screen` fed in record
order that cuts checkpoints with `Screen::serialize`. A real session engine
implements `Engine`:

```rust
impl Engine for MyEngine {
    type Config = MyConfig;
    fn start(config: &MyConfig, size: Size) -> Result<Self, Error> { ... }
    fn restore(config: &MyConfig, checkpoint: &Checkpoint) -> Result<Self, Error> { ... }
    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> { ... }
    fn finish(self) -> Result<TermState, Error> { ... }
}
```

`apply` gets every record in rseq order and returns a checkpoint when it cut
one; the blob inside is the engine's own format. `finish` reads the terminal
with `TermState::capture(screen)`. Then `InProcess::<MyEngine>::new(config,
Restore::Checkpoint)` runs under every test here. An engine in another process
implements `Target` instead, or speaks the subject protocol in `child.rs`.

## What a checkpoint does not restore

`tests/checkpoint.rs` runs the differential from checkpoints over
`Profile::round_trip()`, the generator features that survive the formatter's
round trip, and keeps each loss it found as a named case: a pending wrap, saved
cursors, the inactive screen, kitty keyboard flags, cursor shape, character
protection, left and right margins, origin mode, background-coloured blank
rows at the bottom, soft wraps (which come back as hard breaks and reflow
differently), history when the screen ends in blank rows, styled blank cells
after redraws, and combining marks. When one starts to restore, its test fails
and says so.

## Transcripts

`transcripts/*.rec` are PTY reads in a small framed format (little-endian):

```text
file    = "VREC1\n" cols:u16 rows:u16 record*
record  = "D" delta_us:u32 len:u32 bytes[len]     one read of output
        | "R" delta_us:u32 cols:u16 rows:u16      the PTY was resized
```

Re-record one on Linux or macOS with `python3 scripts/record.py vim
transcripts/vim.rec` (also `htop`, `claude`). The script runs the program on a
fresh PTY with only `TERM`, `LANG`, `PATH` and an empty `HOME` set, types a
fixed script of keys and resizes, and shows htop only the processes it started.
Before committing a recording, check its bytes for anything about the machine
or an account (user and host names, home paths, addresses, tokens).
