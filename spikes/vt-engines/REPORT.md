# Terminal engines for vornd's screen model

Question: could a pure-Rust engine replace libghostty-vt, so vornd no longer
needs a Zig toolchain? Today vornd parses every session through vorn-screen,
which wraps libghostty-vt. Zig costs us exact Zig 0.15.2, a patched-SDK shim on
macOS and a separate build without Ghostty in CI.

**Recommendation: keep libghostty-vt.** alacritty_terminal is the only
pure-Rust engine that comes close. It is fast enough and builds for every
target with no extra toolchain. It still falls short:

- **Correctness:** on adversarial input it matches Ghostty on only 78–88% of
  cells.
- **Resize:** it reflows differently after a resize.
- **Memory:** a full 10k-line history takes 2.2× Ghostty's memory.
- **Features:** it has no screen serializer, and vornd's checkpoints depend on
  one.

Switching would mean rewriting vorn-screen's checkpoint code (1,661 lines),
porting its emulator wrapper and the grid crate (about 4,800 more lines that
call into Ghostty), and accepting visible behaviour changes. wezterm-term is
5–15× slower than Ghostty, is not published, and does not build for iOS. vt100
panics on valid input, does not reflow, and has no link or reply support.

## Engines

| Engine             | Version                                                                                                          | Language   | Unique crates built |
| ------------------ | ---------------------------------------------------------------------------------------------------------------- | ---------- | ------------------: |
| libghostty-vt      | crate 0.2.2 (Ghostty `a887df42c56f`), through vorn-screen's `Emulator` (the product path)                        | Zig, C ABI |                  16 |
| libghostty-vt raw  | same, bare `Terminal`, no vorn-screen wrapper                                                                    | Zig, C ABI |                  16 |
| alacritty_terminal | 0.26.0 (vte 0.15.0)                                                                                              | Rust       |                  35 |
| wezterm-term       | upstream repo at `372548295b0b` (wezterm-term 0.1.0, termwiz 0.24.0); not on crates.io (termwiz there is 0.23.3) | Rust       |                 204 |
| vt100              | 0.16.2                                                                                                           | Rust       |                   8 |

Machine: Apple M2 Pro, 16 GB, macOS. rustc 1.97.0, release builds with
`lto = "fat"` and `codegen-units = 1`, `-j 3`, one process at a time.

## Results

### Throughput (MiB/s, median of 15 timed runs over 8 MiB, interquartile range in brackets)

| Corpus                    | Ghostty (product) | Ghostty raw      | alacritty     | wezterm       | vt100         |
| ------------------------- | ----------------- | ---------------- | ------------- | ------------- | ------------- |
| build log (plain shell)   | 142 [129–144]     | 221 [197–248]    | 105 [99–109]  | 20 [19–22]    | 92 [86–96]    |
| vim (full-screen redraw)  | 25 [25–27]        | 286 [269–338]    | 156 [148–172] | 17 [16–17]    | 131 [126–134] |
| htop (full-screen redraw) | 19 [18–20]        | 129 [122–132]    | 165 [157–167] | 31 [30–31]    | 151 [148–152] |
| agent CLI transcript      | 122 [116–126]     | 253 [249–255]    | 175 [168–192] | 30 [29–30]    | 163 [153–171] |
| seeded generator          | 12.8 [12.3–14.2]  | 22.5 [21.3–24.4] | 58 [53–60]    | 8.5 [8.3–8.6] | 111 [108–112] |

Engine against engine, alacritty runs at 0.47–0.69× Ghostty raw on the build
log, vim and the agent transcript, and at 1.3–2.6× on htop and the seeded
corpus. Against the product path it is faster everywhere except the build
log, where it runs at 0.74×.

On full-screen redraws, the product path is 7–11× slower than raw Ghostty
(vim 25 vs 286). The cost is vorn-screen's own work, not Ghostty's parser:
`Emulator::feed` runs a second parser (`vtparse.rs`) and splits writes around
every hooked sequence. A different engine would not remove that cost unless the
hooks moved into it.

### Resize with a full history (ms per resize, 20 alternating 120x40 ↔ 100x30)

| Ghostty       | Ghostty raw   | alacritty     | wezterm          | vt100                   |
| ------------- | ------------- | ------------- | ---------------- | ----------------------- |
| 4.7 [4.4–6.0] | 4.7 [4.4–6.1] | 2.9 [1.8–6.0] | 14.2 [11.0–21.2] | 0.002 (does not reflow) |

### Memory (32 sessions, 10k-line history, RSS growth per session)

| Corpus                                  | Ghostty  | Ghostty raw | alacritty | wezterm | vt100    |
| --------------------------------------- | -------- | ----------- | --------- | ------- | -------- |
| empty session                           | 139 KiB  | 138 KiB     | 109 KiB   | 72 KiB  | 66 KiB   |
| agent transcript (6 KiB, as recorded)   | 564 KiB  | 561 KiB     | 3,148 KiB | 166 KiB | 386 KiB  |
| 4 MiB build log at 120x40, history full | 10.7 MiB | 10.7 MiB    | 23.5 MiB  | 2.5 MiB | 35.6 MiB |

Ghostty limits history by bytes, not lines. The spike calibrates the budget so
that it keeps at least 10,000 lines; it kept 10,547. The other engines kept
exactly 10,000. wezterm stores lines compactly, while alacritty and vt100 store
a full cell struct for every column. The 3 MiB that alacritty uses after the
short agent transcript was not investigated.

### Correctness against Ghostty (vorn-screen `Emulator`)

The candidate and Ghostty play the same input in lockstep, and the visible grid
is compared cell by cell at every checkpoint. Each cell is a pair: the
percentage of cells that match, and the percentage of checkpoints that match
exactly.

| Corpus (checkpoints)                         | Ghostty raw  | alacritty   | wezterm     | vt100                                |
| -------------------------------------------- | ------------ | ----------- | ----------- | ------------------------------------ |
| build log (33)                               | 100 / 100    | 100 / 100   | 100 / 100   | 100 / 100                            |
| vim, with its resizes (61)                   | 100 / 100    | 99.7 / 98.4 | 99.3 / 41.0 | 99.7 / 98.4                          |
| htop, with its resizes (32)                  | 100 / 100    | 100 / 100   | 98.6 / 43.8 | 99.2 / 96.9                          |
| agent transcript (13)                        | 100 / 100    | 100 / 100   | 100 / 100   | 100 / 100                            |
| seeded, 8 seeds × 256 KiB with resizes (254) | 99.97 / 99.6 | 87.6 / 6.3  | 92.2 / 5.9  | 91.9 / 4.5, panicked in 4 of 8 seeds |
| seeded, same seeds without resizes (124)     | 100 / 100    | 78.4 / 5.6  | 90.7 / 7.3  | 79.3 / 5.6                           |

How to read the seeded rows: once a session diverges it stays diverged, so
the share of identical checkpoints measures how soon the first divergence
comes. Every candidate diverges by its first or second checkpoint (op 16–32).
The low cell-match figures mostly come from that cascade.

Kinds of difference, with what causes them:

- **Line wrapping and reflow after resize.** All three candidates differ from
  Ghostty after vim and htop resize: rows land at different offsets, and
  wezterm keeps rows laid out at the old width. vt100 never reflows.
- **History line count.** After the transcripts resize, wezterm reports no
  history where Ghostty reports some (vim 0 vs 9, htop 0 vs 10).
- **Wide characters and graphemes.** The first divergence in the seeded
  sessions is in rows that hold emoji ZWJ sequences (👩‍👩‍👧), combining
  marks (a + U+0308) and East Asian wide characters: the engines place their
  cells and spacers differently. Cursor positions after them shift with
  them.
- **Line drawing.** vt100 has no DEC special graphics: `lqqk` stays ASCII where
  Ghostty draws `┌──┐`.
- **Colours and attributes.** Most of these follow from rows that have already
  shifted; for example, vim's inverse status line sits on a different row
  after its resize. vt100 also has no strikethrough and only a single
  underline.
- **Hyperlinks.** vt100 has no OSC 8 support. In alacritty and wezterm the
  link spans differ from Ghostty's, mostly on rows that have already
  shifted.
- **OSC titles.** The generator writes a title that never terminates. Ghostty
  takes the text up to the next terminator as the title; all three candidates
  drop it and report no title.
- **Alternate screen.** In the seeded corpus, alacritty and vt100 sometimes
  disagree with Ghostty about whether the alternate screen is active, always
  after the sessions have already diverged.
- **Cursor.** The cursor differs wherever the rows above have shifted; the
  transcripts show no other cause.
- **Panics.**
  - vt100 0.16.2 panics in `screen.rs:870` (`unwrap` on the missing second cell
    of a wide character) on the seeded corpus with resizes. 4 of 8 seeds hit
    it; it never happens without resizes.
  - wezterm-term panics in `Screen::with_phys_lines` once its history ring
    wraps, because it indexes the second half of its deque with absolute
    indices. The adapter avoids it by reading through `lines_in_phys_range`,
    and a regression test covers it.

For an exact list of differences, see `results/compare.jsonl`; each kind
includes up to four examples.

### Build cost and cross-compilation

| Adapter      | Cold release build                                                               | `x86_64-pc-windows-msvc`                                                         | `aarch64-apple-ios`                                                             |
| ------------ | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| vt-ghostty   | 170 s with an empty Zig cache (Ghostty clone, Zig fetch); 56 s with a warm cache | fails: Zig's C++ build of simdutf finds no `cstring` (needs Windows SDK headers) | fails: the libghostty-vt-sys build script panics with "unsupported Rust target" |
| vt-alacritty | 13 s                                                                             | ok                                                                               | ok                                                                              |
| vt-wezterm   | 65 s                                                                             | ok                                                                               | fails: `termios` 0.3.3 has no iOS module (pulled in by termwiz)                 |
| vt-vt100     | 1 s                                                                              | ok                                                                               | ok                                                                              |

Cross targets were checked with `cargo check --release --target <t>`, with only
the rustup std component installed for each target. The Ghostty cold build also
leaves 469 MB in the Zig cache and 332 MB in `target/`.

### Features vornd relies on

| Feature                    | Ghostty (vorn-screen)                                                                | alacritty                                    | wezterm                                    | vt100                                                         |
| -------------------------- | ------------------------------------------------------------------------------------ | -------------------------------------------- | ------------------------------------------ | ------------------------------------------------------------- |
| Snapshot / serialize       | `checkpoint` round-trips exactly; declines while a title stack is pushed (vim, htop) | none                                         | none in wezterm-term                       | `state_formatted`, visible screen only, no history (≤ 2.5 KB) |
| Raw formatter round trip   | not exact (build log and agent rows differ)                                          | –                                            | –                                          | same on all corpora (screen only)                             |
| Alternate screen           | yes                                                                                  | yes                                          | yes                                        | yes                                                           |
| History by absolute line   | yes (`Point::History`/`Screen`, plus grid's own counter)                             | indexable (`Line(-n)`); needs an own counter | yes, natively (`StableRowIndex`)           | only by scrolling the viewport                                |
| History cap                | bytes, not lines                                                                     | lines                                        | lines                                      | lines                                                         |
| Cursor position (CSI 6n)   | yes                                                                                  | yes                                          | yes                                        | no replies at all                                             |
| Device status (CSI 5n)     | yes                                                                                  | yes                                          | yes                                        | no                                                            |
| Primary DA                 | `?62;22c`                                                                            | `?6c`                                        | `?65;4;6;18;22;52c`                        | no                                                            |
| Secondary DA               | yes                                                                                  | yes                                          | yes                                        | no                                                            |
| DECRQM                     | yes                                                                                  | yes                                          | yes                                        | no                                                            |
| XTVERSION                  | yes                                                                                  | no                                           | yes                                        | no                                                            |
| Kitty keyboard flags query | yes                                                                                  | off by default (`Config::kitty_keyboard`)    | no                                         | no                                                            |
| OSC 11 colour query        | no                                                                                   | no                                           | yes                                        | no                                                            |
| OSC 8 links                | yes                                                                                  | yes                                          | yes                                        | no                                                            |
| Title (OSC 0/2)            | yes                                                                                  | yes (event)                                  | yes (default title must be filtered out)   | yes                                                           |
| Synchronized output (2026) | yes                                                                                  | yes; needs a timeout type the host drives    | yes                                        | no                                                            |
| Replies delivered          | inline, as effects from `feed`                                                       | inline, as events                            | through a writer and a thread per terminal | –                                                             |

## What switching to alacritty_terminal would cost in vorn-screen

1. **Checkpoints.** `screen/src/checkpoint.rs` (1,661 lines) draws Ghostty's
   screen cell by cell and records the emulator's tracker state.
   alacritty_terminal has no serializer, so this would be a new serializer over
   its grid, plus a new round-trip test corpus.
2. **The emulator wrapper.** `screen/src/emulator.rs` (1,214 lines) and
   `vtparse.rs` (702 lines) hook sequences around Ghostty's terminal. The hooks
   would need to move to alacritty's `Handler`/event listener, which would also
   remove the second parse that costs the 7–11× above.
3. **Grid.** 7 files in `grid/` call libghostty-vt: absolute-line queries,
   style and link tables, row cache. They would need porting, and they would
   need the absolute line counter that alacritty lacks.
4. **Recovery and engine.** The comparator and fixtures in `recovery` and
   `engine` assume Ghostty, so the JS-reference style fixtures would have to be
   re-recorded.
5. **Behaviour visible to apps and users:**
   - Different reflow after resize.
   - Different grapheme widths for ZWJ sequences and combining marks.
   - A different primary DA.
   - No XTVERSION reply unless added on top; kitty keyboard needs its config
     flag.
   - About 2.2× memory for a full history.
6. **Gains:**
   - No Zig, shim or no-Ghostty fallback build.
   - Windows and iOS builds with only rustup.
   - 13 s cold build instead of 56–170 s.
   - Faster than today's product path on full-screen redraws.

A cheaper step that keeps Ghostty: the product path loses up to 11× to raw
Ghostty on redraws, so that wrapper cost is worth profiling before any engine
switch is considered.

## Methodology

- **Harness.** `crates/harness` (`vt-harness`) drives every engine through
  `vt_api::Engine`. The trait covers feed, resize, visible grid as cells
  (text, width, fg, bg, attributes, underline, link), cursor, history line
  count, history line by index, title, alternate screen, replies and rebuild
  from a snapshot.
- **Corpora:**
  - _build log_: synthetic compiler output at 120x40, fed in 4 KiB chunks.
  - _vim_, _htop_ and _agent CLI transcript_: the recordings in
    `packages/core/crates/recovery`.
  - _seeded_: `recovery`'s `Generator` with `Profile::mixed()`.
- **Throughput.** The corpus is repeated to 8 MiB at its first recorded size,
  with resizes dropped so that history reflow does not dominate. There is one
  warm-up run, then 15 timed runs. A new engine is created outside the timer
  for each run, and the 10k-line history is on.
- **Memory.** Each session is fed the corpus as recorded, with its resizes. RSS
  comes from `ps`, taken before, after creating the empty sessions, and after
  feeding them.
- **Correctness.** The transcripts are compared at every record. The build log
  is compared every 16 chunks over 2 MiB. The seeded corpus is compared every 16
  ops for seeds 1–8 at 256 KiB, once with resizes and once without (the
  `seeded-fixed` corpus).
  - History is capped at 10,000 lines on both sides before the counts are
    compared.
  - Blank cells compare only background, inverse and link. Spacer cells compare
    only width.
  - A panic in the candidate engine is caught, counted as "engine panicked",
    and ends that session.
- **Features.** Each feature is probed with fixed sequences; see
  `crates/harness/src/features.rs`.

## Reproduce

```sh
cd spikes/vt-engines
scripts/fetch-wezterm.sh                       # pins wezterm's upstream into spikes/.vt-vendor
cargo build --release -j 3 -p vt-harness       # Ghostty needs Zig 0.15.2 on PATH
scripts/run-all.sh                             # about 2 minutes, writes results/*.jsonl
cargo test --release -j 3 --workspace
```

## Raw numbers

- `results/throughput.jsonl`: each run's rate, plus min, quartiles, median and
  max.
- `results/memory.jsonl`
- `results/resize.jsonl`
- `results/compare.jsonl`: per-kind counts and examples.
- `results/features.jsonl`
- `results/build.txt`

## Open questions

- Whether vornd can live with Ghostty's byte-budget history, or should switch
  it to a line cap. This affects every engine comparison of memory.
- Whether the wrapper slowdown on redraws (7–11×) shows up in real sessions.
  Profiling `Emulator::feed` on the vim transcript is the next measurement.
- alacritty's 3 MiB per session after a short transcript with resizes.
