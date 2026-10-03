# @vornrun/core

Vorn's Rust core, loaded into the server as `vorn_core.node` through
[napi-rs](https://napi.rs). It parses terminal output with
[libghostty-vt](https://crates.io/crates/libghostty-vt), Ghostty's VT engine.

Off by default. The server only loads it when started with `VORN_CORE=native`,
and falls back to the JS path, with a warning in the log, when the binary is
missing or will not load. `VORN_CORE_PATH` points it at a specific binary.

## Build

Needs a Rust toolchain and [Zig 0.16](https://ziglang.org/download/), which
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
# [core] native: hello server from vorn-core 0.7.5
```
