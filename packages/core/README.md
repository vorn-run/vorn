# @vornrun/core

Vorn's Rust core: `vornd`, the server every client connects to, `vorn-sessiond`,
which holds the terminal sessions, and `vorn`, the command line. Terminals run
in vornd, which parses their output with
[libghostty-vt](https://crates.io/crates/libghostty-vt), Ghostty's VT engine,
and keeps each one's screen model, scrollback and agent status.

## Layout

A Cargo workspace of plain crates, whose tests and benchmarks are ordinary
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
| `crates/vornd` (`vornd`)                      | The server: the endpoint every client uses, its session holder and every call               |
| `crates/sessiond` (`vorn-sessiond`)           | Holds terminal sessions and their output so the app can reconnect                           |
| `crates/sessiond-wire` (`vorn-sessiond-wire`) | The socket protocol between vornd and vorn-sessiond                                         |
| `crates/cli` (`vorn-cli`)                     | The `vorn` command                                                                          |

```sh
cargo test --workspace         # the logic, on any platform
cargo bench -p vorn-analysis   # Criterion
```

## vornd

`vornd --data-dir DIR` is the Vorn server for that data directory: its
database, the port and credential it publishes, and the session holder's home.
The app starts it; `vorn server serve` runs it from a terminal.

```sh
cargo run -p vornd -- --data-dir /tmp/vorn-dev --port 0   # prints {"port":N,"protocol":1}
curl http://127.0.0.1:N/vornd/health                      # its sessions, holder and hooks
```

With `--sessiond` it keeps a session holder under the data directory and
serves grid mode, the native app's attachment to sessions as frames of their
screens: on a user-only local socket (`$VORN_HOME/run/vornd-grid-<pid>.sock`,
or a named pipe on Windows) that its ready line names as `"grid"`.
`--log-file` and `VORND_LOG` control the log. Clients see the version in the
`Vornd-Protocol` header on the WebSocket upgrade.

For the sessions its session holder keeps, vornd answers the terminal calls
(`terminal:attach`, `write`, `resize`, `readScrollback`, `readOutput`) and
streams their output as version 2 bytes frames, which name the records they
carry, with `terminal:resized` between them in record order. A client that
attaches with its cursor continues without a snapshot while vornd's tail or
the holder's ring still has everything after it, across a vornd restart too;
otherwise it gets a snapshot and the reason. `--debug-spawn` lets a test start
a session with `vornd:spawn`.

## Build

Needs a Rust toolchain and [Zig 0.16.0](https://ziglang.org/download/), which
builds libghostty-vt from source on the first build.

```sh
yarn build:core                                 # release build, writes packages/core/vornd, vorn-sessiond and vorn
yarn workspace @vornrun/core build --debug      # unoptimized
yarn workspace @vornrun/core build --no-ghostty # vornd without libghostty-vt, no Zig needed
```

`yarn dist` ships the binaries under `resources/vornd`. The tests that start
vornd need them too: run `yarn build:core` before `yarn test`.
