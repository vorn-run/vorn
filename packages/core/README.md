# @vornrun/core

Vorn's Rust core, loaded into the server as `vorn_core.node` through
[napi-rs](https://napi.rs). It parses terminal output with
[libghostty-vt](https://crates.io/crates/libghostty-vt), Ghostty's VT engine.

The server's terminals run on it: each one's screen model, scrollback and
history framing on a thread of its own (`TerminalPipeline`), agent status from
its output (`Analyzer`), and every git command the server runs (`gitRun`). A
server whose binary is missing or will not load keeps running: its terminals
are drawn and recorded but have no screen model or agent status, git runs as a
child process, and the reason is in the log and on **Settings › Experimental**.
`VORN_CORE_PATH` points the server at a specific binary.

## Layout

A Cargo workspace. The logic is in plain crates with no napi in them, so it can
later serve a daemon or the native UI, and its tests and benchmarks are ordinary
Rust binaries:

| Crate                                         | What                                                                                        |
| --------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `crates/screen` (`vorn-screen`)               | A terminal's screen on libghostty-vt: feed, title, cwd, serialize                           |
| `crates/analysis` (`vorn-analysis`)           | Stripped line ring, bracketed paste and status patterns                                     |
| `crates/pipeline` (`vorn-pipeline`)           | A terminal's thread: screen, scrollback ring, history frames and checkpoint bodies          |
| `crates/term-proto` (`vorn-term-proto`)       | Wire types: record headers and cursors, the screen mirror's rows, `row_fmt` 1, bytes frames |
| `crates/term-mirror` (`vorn-term-mirror`)     | A client's copy of a screen, kept from snapshots and deltas with no parser                  |
| `crates/grid` (`vorn-grid`)                   | Grid mode's server half: render updates, row cache, tables, credits, history                |
| `crates/grid-client` (`vorn-grid-client`)     | A headless grid client: the mirror behind grid mode's framing                               |
| `crates/engine` (`vorn-engine`)               | vornd's session engine: parses each session's records, cuts checkpoints, recovers from them |
| `crates/vornd` (`vornd`)                      | A daemon in front of the server: forwards its calls, keeps sessiond and runs its sessions   |
| `crates/sessiond` (`vorn-sessiond`)           | Holds terminal sessions and their output so the app can reconnect                           |
| `crates/sessiond-wire` (`vorn-sessiond-wire`) | The socket protocol between vornd and vorn-sessiond                                         |
| `.` (`vorn-core`)                             | The napi adapters the server loads as `vorn_core.node`                                      |

```sh
cargo test --workspace --exclude vorn-core   # the logic, on any platform
cargo bench -p vorn-analysis                 # Criterion, without napi
```

`gitRun` runs each git command on a thread pool instead of on Node's event
loop. The logic lives in `crates/vorn-git`, a plain crate: git as a
child process with `execFileSync`'s limits and error messages, plus a
[gix](https://crates.io/crates/gitoxide) fast path that answers the
`rev-parse` queries in-process where it can match git byte for byte.

## vornd

`vornd` is a separate binary that answers the server's WebSocket and HTTP
endpoint and forwards everything to the Node server, unchanged. It runs only
when started by hand; nothing in the app starts it yet.

```sh
cargo run -p vornd -- --upstream 127.0.0.1:50091   # prints {"port":N,"protocol":1}
curl http://127.0.0.1:N/vornd/health                # server reachable, calls per group
yarn test:conformance                               # the RPC test files, through vornd
```

With `--sessiond` and `--home` it also serves grid mode, the native app's
attachment to sessions as frames of their screens: on a user-only local socket
(`$VORN_HOME/run/vornd-grid-<pid>.sock`, or a named pipe on Windows) that its
ready line names as `"grid"`.

It listens on loopback only. `--groups git=shadow` (or `VORND_GROUPS`) sets a
group's mode: `forward`, `shadow`, or `native` once a group has a native
implementation. A group is the method name before its first colon. `--log-file`
and `VORND_LOG` control the log. Clients see the version in the
`Vornd-Protocol` header on the WebSocket upgrade, and vornd closes a connection
whose server speaks a protocol version it does not know.

For the sessions its session holder keeps, vornd answers the terminal calls
itself (`terminal:attach`, `write`, `resize`, `readScrollback`, `readOutput`)
and streams their output as version 2 bytes frames, which name the records they
carry, with `terminal:resized` between them in record order. A client that
attaches with its cursor continues without a snapshot while vornd's tail or
the holder's ring still has everything after it, across a vornd restart too;
otherwise it gets a snapshot and the reason. Calls for any other session go to
the server as before. `--debug-spawn` lets a test start a session through vornd
with `vornd:spawn`; `yarn test:conformance` builds the holder too and runs the
terminal and attach files that way.

## Build

Needs a Rust toolchain and [Zig 0.15.2](https://ziglang.org/download/), which
builds libghostty-vt from source on the first build.

```sh
yarn build:core                                 # release build, writes packages/core/vorn_core.node
yarn workspace @vornrun/core build --debug      # unoptimized
yarn workspace @vornrun/core build --no-ghostty # without libghostty-vt, no Zig needed
```

`yarn dist` ships the binary at `resources/core/vorn_core.node` when it has
been built, and skips it otherwise. The tests need it too: run `yarn build:core`
before `yarn test`.

## Try it

```sh
yarn build:core
yarn workspace @vornrun/server dev
# [core] native 0.7.5
```
