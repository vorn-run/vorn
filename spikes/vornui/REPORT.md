# vornui spike: GPUI or a thin UI layer of our own

The question: should the Windows/Linux UI be built on GPUI, or on vornui, a
thin layer of our own made only from general crates (winit, wgpu,
cosmic-text + swash, taffy, AccessKit, resvg)?

To answer it, both prototypes draw the same two things:

- today's main screen: top bar with icon buttons, the terminals/tasks/workflows
  pill switch, sessions chip, gold logo, and the composer with chips and the
  send button, using the values from `src/renderer/theme.css` and the
  components;
- a terminal view drawing vornd's cell grid (`grid-client`, `term-mirror`,
  `term-proto`) for 8 and 32 busy panes, fed by a test vornd started in a
  temp directory.

Both share one harness (`shared/`) for the daemon, load, grid mirroring,
layout values, metrics and the bench loop. Only the UI layer differs.

## Recommendation

**Build vornui.** It is the better base for Vorn's Windows and Linux UI,
with two conditions taken on as the first work:

1. a GPU-less (software adapter) rendering path that is not much slower
   than GPUI's. On Windows' software adapter today, vornui renders the
   8-pane grid at about 3 fps against GPUI's 27 (see Windows below);
2. latency pacing that matches GPUI's tight tail.

Why vornui:

- **Cheaper at the same frame rate.** On the Mac it uses about half of
  GPUI's CPU at 8 and 32 busy panes (24% and 15%, against 45%), with 3–6×
  lower CPU frame time and a lower peak RSS (28 MB, against 39–43 MB).
- **Half the dependency tree, and only general crates.** 175 crates
  against 328. Nothing has to be patched: GPUI's headless and IME hooks
  needed 83 lines of patches to a pinned checkout, and its Windows
  backends are split (DX11 for windows, wgpu for offscreen).
- **Control over the hard parts Vorn cares about.** Those are the terminal
  grid, IME composition in a pane, and the accessibility tree, and vornui
  owns all three. The layer is 2,154 lines and the app on top is 767 lines,
  against 1,012 for the same app on GPUI.

GPUI wins on things that are real but fixable on our side:

- a tighter latency tail and faster cold start (95 ms against 236 ms on the
  Mac);
- a far faster software-rendering path on the Windows runner: 27 fps
  against 3 fps at 8 panes, with a keystroke-to-pixel p50 of 119 ms
  against 674 ms. Its memory there is the cost: 574 MB peak RSS against
  85 MB, and a 5.8 s cold start against 0.47 s;
- a widget and focus model we would otherwise have to write.

These are the risks that would change the answer:

- vornui's software-adapter rendering cannot be brought close to GPUI's;
- the widget work below grows past a few thousand lines.

## Screens

Today (Electron, 1440×900 at 2×; onboarding skipped, sidebar closed, no
sessions so no sessions chip, agent chip relabelled "Agent"):

![today](results/mac/today.png)

| vornui (macOS) | GPUI (macOS) |
|---|---|
| ![vornui main](results/mac/vornui-main.png) | ![gpui main](results/mac/gpui-main.png) |
| ![vornui grid 8](results/mac/vornui-grid-8.png) | ![gpui grid 8](results/mac/gpui-grid-8.png) |
| ![vornui grid 32](results/mac/vornui-grid-32.png) | ![gpui grid 32](results/mac/gpui-grid-32.png) |
| ![vornui ime preedit](results/mac/vornui-ime-preedit.png) | ![gpui ime preedit](results/mac/gpui-ime-preedit.png) |

| vornui (Windows) | GPUI (Windows) |
|---|---|
| ![vornui main win](results/windows/vornui-main.png) | ![gpui main win](results/windows/gpui-main.png) |
| ![vornui grid 8 win](results/windows/vornui-grid-8.png) | ![gpui grid 8 win](results/windows/gpui-grid-8.png) |

HiDPI shots at 1× and 1.5× are in `results/*/*-main-1x.png` and
`*-main-1.5x.png`; both prototypes lay out in logical pixels and
rasterize text and icons at the device scale.

GPUI's mono cell comes out slightly wider than vornui's, from the Menlo
advance as GPUI measures it, so its lines wrap a few columns earlier. Each
is consistent within its own layout.

## Results

### macOS: Apple M2 Pro, Metal, 1440×900 at 2×

| | vornui | GPUI |
|---|---|---|
| **8 busy panes**, frame p50 / p95 / p99 | 1.05 / 1.87 / 2.39 ms | 3.40 / 4.04 / 4.18 ms |
| keystroke-to-pixel p50 / p95 / p99 | 9.66 / 10.40 / 11.32 ms | 8.24 / 9.22 / 10.25 ms |
| fps, lost probes | 104.7, 0 | 108.7, 0 |
| CPU | 23.7% | 44.5% |
| peak RSS / phys footprint | 27.6 / 78.8 MB | 39.3 / 52.4 MB |
| **32 busy panes**, frame p50 / p95 / p99 | 0.57 / 0.71 / 0.89 ms | 3.28 / 3.53 / 3.69 ms |
| keystroke-to-pixel p50 / p95 / p99 | 9.68 / 10.21 / 10.32 ms | 8.28 / 9.08 / 9.42 ms |
| fps, lost probes | 104.0, 4 | 109.4, 4 |
| CPU | 15.1% | 45.4% |
| peak RSS / phys footprint | 28.9 / 78.1 MB | 43.1 / 53.7 MB |
| static redraw, main 2× p50 / p99 | 1.43 / 1.76 ms ¹ | 0.21 / 0.40 ms |
| static redraw, grid 8 p50 / p99 | 1.56 / 1.75 ms ¹ | 1.19 / 1.43 ms |
| static redraw, grid 32 p50 / p99 | 1.54 / 2.88 ms ¹ | 1.83 / 2.05 ms |
| cold start to first frame of 8 panes, p50 of 5 | 236 ms | 95 ms |
| IME (Japanese): preedit drawn, commit reaches the pty | yes, `$ 日本語` | yes, `$ 日本語` |
| a11y tree (main / grid) | 20 / 13 nodes, terminals carry screen text | 19 / 13 nodes, terminals carry screen text |
| HiDPI 1× / 1.5× / 2× | yes | yes |
| release binary | 9.1 MB | 10.7 MB ² |
| lines we own: UI layer / app | 2,154 / 767 | 83 lines of patches / 1,012 |
| dependency crates | 175 (layer alone 145) | 328 |

¹ vornui's redraw loop waits for the previous frame's GPU work, as a
swapchain would, so the number is GPU throughput for the 2880×1800 target.
GPUI's Metal path commits without waiting, so its number is CPU time only.
Before the wait was added, vornui's CPU-only redraws were 0.41 ms (main),
0.38 ms (grid 8) and 0.38 ms (grid 32) at p50. In the paced benches the
GPU is idle by the next frame, so the wait costs nothing there.
Frame p50 was 1.06 and 0.63 ms before the change, and 1.05 and 0.57 ms
after.

² GPUI is built with its test-support feature, which carries the headless
platform.

### Windows: windows-2022 runner, Hyper-V, no GPU (WARP)

| | vornui | GPUI |
|---|---|---|
| **8 busy panes**, frame p50 / p95 / p99 | 339 / 370 / 388 ms ¹ | 13.8 / 37.7 / 48.9 ms |
| keystroke-to-pixel p50 / p95 / p99 | 674 / 727 / 745 ms | 119 / 711 / 723 ms |
| fps, lost probes | 2.95, 0 | 27.2, 1 |
| CPU | 214% | 307% |
| peak RSS / private bytes | 85 / 269 MB | 574 / 939 MB |
| **32 busy panes**, frame p50 / p95 / p99 | 513 / 580 / 604 ms ¹ | 15.2 / 58.4 / 99.2 ms |
| keystroke-to-pixel p50 / p95 / p99 | 1,034 / 1,109 / 1,152 ms | 624 / 994 / 1,004 ms |
| fps, lost probes | 1.94, 0 | 28.8, 8 |
| CPU | 136% | 249% |
| peak RSS / private bytes | 86 / 267 MB | 737 / 1,097 MB |
| static redraw, main 1× / 1.5× / 2× p50 | 6.6 / 12.9 / 21.3 ms ¹ | 1.9 / 1.9 / 1.9 ms |
| static redraw, grid 8 p50 / p99 | 235 / 256 ms ¹ | 11.4 / 169 ms |
| static redraw, grid 32 p50 / p99 | 276 / 355 ms ¹ | 7.3 / 142 ms |
| cold start to first frame of 8 panes, p50 of 5 | 471 ms (389–646) | 5,758 ms (3,773–6,547) |
| IME (Japanese): preedit drawn, commit reaches the pty | yes, `$ 日本語` | yes, `$ 日本語` |
| a11y tree (main / grid) | 20 / 13 nodes | 19 / 13 nodes |
| release exe | 12.0 MB | 19.8 MB ² |

¹ ² as in the macOS table.

The runner has no GPU, so both prototypes rasterize through Windows' software
DX12 adapter (WARP). These are numbers for a GPU-less machine (a VM or a
remote session), not for a desktop with a graphics card. Frame and redraw
times are not comparable across the two prototypes here: vornui waits for
the previous frame, while GPUI queues without waiting, so its frame time is
CPU-only. Its backlog shows up instead in latency (the p95 at 8 panes) and
in memory, which grows to 0.6–0.7 GB of RSS. Keystroke-to-pixel latency,
fps, memory and cold start are comparable. Even so, GPUI turns out several
times more frames and has a much lower median latency on WARP. vornui's GPU
work per frame is about 20× GPUI's on this adapter, and its grid frames are
software-rendered at about 3 fps. GPUI's cold start is slow here because it
creates the windowed DirectWrite platform and compiles its wgpu pipelines on
WARP. GPUI on Windows draws offscreen with its wgpu renderer, not the DX11
renderer it uses for real windows.

## How it was measured

- `scripts/measure.sh <binary> <name> <out>` runs every mode for one
  prototype. The CI workflow `.github/workflows/vornui-spike.yml` runs it on
  windows-2022 and uploads the JSON and PNGs.
- **Daemon.** Each run starts the released vornd (v0.8.0-beta.7) with its
  own temp `HOME` and `--data-dir`, and kills it and its sessiond on exit.
- **Load.** Busy panes run a producer that writes colored lines at 1 MB/s, so
  every pane has a new screen every frame. The probe pane echoes keys in raw
  mode, and screenshots use a few idle screens of a build log.
- **Frame time.** Each frame is timed from its start to the GPU submit,
  covering pulling dirty panes, building, layout and encoding. The bench
  paces frames like a 120 Hz display and renders offscreen into a texture,
  with no window.
- **Keystroke-to-pixel.** The driver types into pane 0 through the
  prototype's own input path. Each key arms a probe on the grid client, and
  its latency ends at the submit of the first frame that draws its glyph.
- **CPU** is user+sys over the 20 s run, for the prototype process only (not
  vornd or the producers). **RSS** is peak resident; on the Mac, the
  physical footprint is also reported.
- **Cold start** is a fresh process with the daemon already up, through
  GPU init, fonts and grid attach, to its first complete frame of 8 panes.
- **IME.** Composition events are injected the way the OS IME would
  deliver them: through winit `Ime` events for vornui, and through GPUI's
  platform input handler. No OS IME window is driven.
- **a11y.** vornui builds an AccessKit tree; GPUI uses its accessibility
  tree, with the test window acting as if a screen reader were connected.
  Both dumps are in `results/*/*-a11y.json`.
- **GPUI** is a sparse checkout at a pinned commit (`gpui/fetch_zed.sh`).
  `gpui/patch_zed.py` makes four test-support changes:
  - present frames headless;
  - expose the IME input handler;
  - activate the a11y tree;
  - set the test window's scale, and enable DX12 for its wgpu headless
    renderer.

  On Windows, the prototype takes its text system from the windowed
  platform, because the headless one has no text system.

Caveats:

- IME and screen-reader support are verified as plumbing, not with a real
  IME or screen reader.
- Linux was not run. vornui's winit has X11 and Wayland disabled to keep
  the build small, and GPUI on Linux is its wgpu renderer.
- The earlier windowed GPUI numbers (1.3 ms p50 typing, 32% CPU, 106 MB at 8
  panes) came from a different harness and are not comparable with these.

## What vornui still needs to be complete

- Real OS windows on all three platforms:
  - surface presentation and the event loop;
  - the system IME window position (`set_ime_cursor_area`);
  - the AccessKit adapter wired to the window, with actions (focus, press,
    set value) as well as the tree.
- Software-adapter performance. On WARP, a grid frame takes about 235 ms
  of GPU work, and even the main screen takes 21 ms at 2×. GPUI's CPU-side
  redraws there are 2–11 ms. Profile the fragment work per pixel (rounded
  quads and linear sampling of the mask atlas) and the sprite path, and
  batch glyph runs.
- Latency pacing: present right after input instead of on the next slot.
  Today the tail is good once frames are bounded, but the p50 is about
  1.4 ms behind GPUI's.
- Widgets: text input with selection and clipboard, scrolling, lists,
  menus and popovers, tooltips, focus and tab order, hover and press
  states, and animation.
- Terminal: selection, scrollback scrolling, links, and wide and combining
  glyphs beyond what the spike draws.
- Rendering:
  - damage tracking, so only dirty panes are re-encoded;
  - an atlas eviction policy;
  - color emoji;
  - subpixel positioning checks against today's text.
- Linux: X11 and Wayland, fontconfig fallback, and portal IME.
- A theming layer that reads the same tokens as `theme.css`.
