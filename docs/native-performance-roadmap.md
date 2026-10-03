# Native performance roadmap

Where Vorn spends CPU today, what "native" and "GPU" should mean for it, and the
order to do it in. Every phase stands alone and is measurable.

## Where the time goes now

Drawing is already on the GPU: every terminal mounts `@xterm/addon-webgl`
(`src/renderer/lib/terminal-registry.ts:414-431`). The expensive work is not
pixels. It is parsing, copying and blocking in the Node server and the Electron
main process.

| #   | Hotspot                                                                    | Where                                                               | Cost                                                                                                      |
| --- | -------------------------------------------------------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| 1   | ANSI strip + status regexes on every raw PTY chunk                         | `packages/server/src/pty-manager.ts:792-862`, `ansi-strip.ts`       | Four regexes per chunk; `applyCarriageReturns` is O(n²) per line, and spinners/progress bars are all `\r` |
| 2   | Second VT parse of all output in headless xterm                            | `packages/server/src/terminal-screen.ts`                            | Every byte parsed twice; only consumer is the history checkpoint                                          |
| 3   | Synchronous git on the server event loop                                   | `packages/server/src/git-utils.ts:22-38` (35 sites, `execFileSync`) | A slow `git diff` stalls PTY flushes for every session                                                    |
| 4   | Device pane: PNG decode → resize → PNG encode → base64, 2 fps, main thread | `src/main/device-registry.ts:1177-1230`, `useDeviceFrame.ts`        | ~3 MB PNG round trip twice a second while a simulator is visible                                          |
| 5   | Per-frame `getBoundingClientRect` for every terminal                       | `TerminalHost.tsx:56-60`, `terminal-registry.ts:704-749`            | Layout read on every rAF, N terminals                                                                     |
| 6   | Shiki with the JS regex engine on the main thread                          | `code-editor/shiki.ts:97-120`                                       | Whole-file tokenize, no worker                                                                            |
| 7   | Flush fan-out with no size cap or backpressure                             | `pty-manager.ts:650-763`, `broadcast.ts:168-180`                    | One flush = emit + scrollback + screen + history + bell scan; unbounded in an 8 ms window                 |

Existing measurement tests: `tests/write-path-throughput.process.test.ts`,
`tests/history-cost.process.test.ts`, `tests/terminal-screen-memory.process.test.ts`.
Extend these rather than adding a new harness.

## What "native" should mean here

**Recommended: a Rust core, one UI.** Put the hot loop in a Rust crate
(`packages/core`), expose it to Node through napi-rs today, and keep the React
renderer. The crate is the piece that outlives any UI choice: a later Tauri
shell, a Swift or WinUI app, or the CLI all link the same core.

**Not recommended now: rewriting the UI per platform.** The renderer is ~69k
lines (`src/renderer`) and the web PWA (`packages/web`) reuses it. Three native
UIs means three copies of the grid, workflow canvas, diff review, task board,
artifacts viewer and command palette. None of the hotspots above live there.

**GPU, concretely.** Terminal text is already GPU-drawn. The remaining GPU
wins are:

- Device frames as a hardware-decoded video stream instead of PNG polling.
- A WebGPU/wgpu terminal renderer only if measurements show WebGL is the
  bottleneck at high terminal counts. Today it is not.
- Shiki tokenization is CPU work; move it off-thread, not onto the GPU.

## Phases

### Phase 0: measure (1 week)

- Add a `yarn bench` script that runs the three `*.process.test.ts` files and
  prints ms per MB of output, server event-loop lag p99, and renderer frame
  time with 1, 8 and 32 terminals.
- Record baselines in this file.

### Phase 1: Rust core via napi-rs (2 to 3 weeks)

Crate `packages/core` (Rust), binary `vorn_core.node`, built with napi-rs and
prebuilt per platform like `libsql` already is (`electron-builder.yml`
`asarUnpack`).

Move, in this order, each behind the existing TypeScript interface so tests
keep passing:

1. **VT state machine + screen model** (replaces `@xterm/headless` on the
   server, hotspot 2). Use the `vte` or `alacritty_terminal` crate. Export
   `feed(bytes)`, `serialize()`, `title()`, `cwd()`.
2. **Output analysis** (hotspot 1): ANSI stripping, carriage-return
   resolution, line ring, bracketed-paste and status regexes. One pass over
   the bytes, driven by the same VT parser as item 1 so output is parsed
   once, not twice.
3. **History log framing** (`history/log.ts`): UTF-8 + CRC-32 + append on a
   Rust thread; ship an mpsc channel instead of a 250 ms JS timer.
4. **Flush pipeline**: cap a flush at 64 KB, apply `bufferedAmount`
   backpressure in `broadcast.ts`, and move the scrollback ring to the core.

Exit criteria: write-path throughput test ≥ 5× faster; server event-loop lag
p99 < 2 ms under a 50 MB/s synthetic PTY burst.

### Phase 2: unblock the event loop (1 week)

- `gitExec` becomes async (`execFile`), with a small concurrency pool, or use
  the `gix` (gitoxide) crate inside the core for diff/status/numstat.
- Same for `worktree-inventory.ts`, `ide-detector.ts`, `rpc-client.ts`.

Exit criteria: a 500 KB `git diff` no longer delays PTY flushes (measurable in
the bench as p99 lag during diff polling).

### Phase 3: device frames as video (2 weeks, macOS first)

- Replace the 2 fps PNG poll with `idb_companion`'s video stream
  (`video_stream` RPC, H.264), decode with `VideoDecoder` (WebCodecs, hardware
  accelerated) in the renderer, draw to a canvas.
- Fallback: keep PNG polling but do decode and resize in the core with
  `image` + `fast_image_resize` on a worker thread, and send raw RGBA over
  a `SharedArrayBuffer` rather than base64 PNG.

Exit criteria: main-process CPU with a visible simulator < 5 %; ≥ 30 fps.

### Phase 4: renderer trims (1 week)

- Replace the per-frame `getBoundingClientRect` loop with a
  `ResizeObserver` per slot plus a single rAF when something moved.
- Shiki: load the WASM Oniguruma engine in a Web Worker; post tokens back.
- Diff sidebar: virtualize lines (`@tanstack/react-virtual`).
- Share one WebGL context policy: pause the WebGL addon for off-screen
  terminals (the IntersectionObserver in `TerminalSlot.tsx` already knows).

### Phase 5 (optional): a lighter shell

Once the core exists, the Electron main process is thin: window management,
IPC, updater, tray, device companion. That is where Tauri 2 (Rust shell +
system WebView) becomes a drop-in: same renderer bundle, same core crate,
~10× smaller download, lower idle memory. Decide after Phase 1 with real
numbers. A full native UI (SwiftUI/WinUI/GTK) is the step after that and
should be driven by a feature the web renderer cannot do, not by speed.

## Build and packaging notes

- napi-rs produces one `.node` per target; publish with
  `@napi-rs/cli` prebuilds and list them in `asarUnpack` next to `libsql`.
- `node-pty` versions differ between root (`1.2.0-beta.13`) and server
  (`1.2.0-beta.15`); align them before adding a third native module.
- CI already builds on ubuntu, macos and windows-2022; add `cargo build
--release` and cache `target/`.
- Keep the TypeScript implementations behind a feature flag
  (`VORN_CORE=js|native`) until the bench shows parity, then delete them.
