# @vornrun/core

Vorn's Rust core, loaded into the server as `vorn_core.node` through
[napi-rs](https://napi.rs). It parses terminal output with
[libghostty-vt](https://crates.io/crates/libghostty-vt), Ghostty's VT engine.

Off by default. Each piece it can take over has a switch in **Settings ›
Experimental**, which applies to terminals opened after it is turned on.
`VORN_CORE=native` in the server's environment turns every switch on and
`VORN_CORE=js` turns every switch off. A switch that is on with a binary that is
missing or will not load stays on the JS path, says why in the log and on the
settings page. `VORN_CORE_PATH` points the server at a specific binary.

## Layout

A Cargo workspace. The logic is in plain crates with no napi in them, so it can
later serve a daemon or the native UI, and its tests and benchmarks are ordinary
Rust binaries:

| Crate                                     | What                                                                               |
| ----------------------------------------- | ---------------------------------------------------------------------------------- |
| `crates/screen` (`vorn-screen`)           | A terminal's screen on libghostty-vt: feed, title, cwd, serialize                  |
| `crates/analysis` (`vorn-analysis`)       | Stripped line ring, bracketed paste and status patterns                            |
| `crates/pipeline` (`vorn-pipeline`)       | A terminal's thread: screen, scrollback ring, history frames and checkpoint bodies |
| `crates/term-proto` (`vorn-term-proto`)   | Wire types: record headers and cursors, the screen mirror's rows, `row_fmt` 1      |
| `crates/term-mirror` (`vorn-term-mirror`) | A client's copy of a screen, kept from snapshots and deltas with no parser         |
| `crates/vornd` (`vornd`)                  | A daemon in front of the server; forwards everything for now                       |
| `crates/sessiond` (`vorn-sessiond`)       | Holds terminal sessions and their output so the app can reconnect                  |
| `.` (`vorn-core`)                         | The napi adapters the server loads as `vorn_core.node`                             |

```sh
cargo test --workspace --exclude vorn-core   # the logic, on any platform
cargo bench -p vorn-analysis                 # Criterion, without napi
```

Git is switched on its own, from Settings › Experimental › Native Git, or with
`VORN_GIT=native|js`, which wins over the setting. On, every git command the
server runs goes to `gitRun`, which runs it on a thread pool instead of on
Node's event loop. The logic lives in `crates/vorn-git`, a plain crate: git as a
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

It listens on loopback only. `--groups git=shadow` (or `VORND_GROUPS`) sets a
group's mode: `forward`, `shadow`, or `native` once a group has a native
implementation. A group is the method name before its first colon. `--log-file`
and `VORND_LOG` control the log. Clients see the version in the
`Vornd-Protocol` header on the WebSocket upgrade, and vornd closes a connection
whose server speaks a protocol version it does not know.

## Build

Needs a Rust toolchain and [Zig 0.15.2](https://ziglang.org/download/), which
builds libghostty-vt from source on the first build.

```sh
yarn build:core                                 # release build, writes packages/core/vorn_core.node
yarn workspace @vornrun/core build --debug      # unoptimized
yarn workspace @vornrun/core build --no-ghostty # without libghostty-vt, no Zig needed
```

`yarn dist` ships the binary at `resources/core/vorn_core.node` when it has
been built, and skips it otherwise.

## Try it

```sh
yarn build:core
VORN_CORE=native yarn workspace @vornrun/server dev
# [core] native 0.7.5
```
