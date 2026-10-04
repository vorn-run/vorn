---
name: napi-boundary
description: Rules for the napi-rs adapter layer in packages/core/src (vorn_core.node) and the TypeScript that loads it. Use when adding or changing a native export, or wiring a native path into the Node server behind a switch.
---

# The napi boundary

`vorn-core` (`packages/core/src`) is a thin adapter over plain crates. It is
loaded into the Node server as `vorn_core.node` by
`packages/server/src/native-core.ts`. Everything here exists to keep one
property: **a native path can only make the server faster, never take it
down.**

## What lives where

| Layer                           | Contains                                                                |
| ------------------------------- | ----------------------------------------------------------------------- |
| `crates/<name>` (`vorn-<name>`) | All logic, its error enum, unit/property tests, Criterion benches       |
| `src/<name>.rs`                 | `#[napi]` types and functions: convert, bound, map errors. Nothing else |
| `packages/server/src/...`       | The switch, the JS fallback, logging why the native path is off         |

If an adapter function grows a loop or a branch on domain data, that logic
belongs in the crate.

## Rules

1. **No panic reaches Node.** Every sync export is
   `#[napi(catch_unwind)]`. An `async` export runs its work in
   `tokio::task::spawn_blocking` and maps the `JoinError`, as `src/git.rs`
   does. An uncaught panic unwinding into Node aborts the server and every
   terminal it hosts.
2. **Never block the event loop.** Sync exports must be cheap and bounded
   (feed one flush, read a field). Anything that can take milliseconds (git,
   history search, bulk analysis, disk) is `#[napi] pub async fn` returning a
   promise, with concurrency bounded by a `Semaphore` in a `LazyLock`.
3. **Cross the boundary per flush, not per chunk.** Batch bytes on the JS side
   and pass one `Buffer` (or one string) per flush. Return what the server
   needs from that flush in one object.
4. **Bytes as bytes.** Prefer `Buffer` / `&[u8]` for terminal data; take a
   `String` only where the JS side already has one, and offer both when both
   callers exist (`feed` and `feed_bytes` in `src/screen.rs`).
5. **Errors become messages.** Map each crate error with a `to_napi` helper to
   `napi::Error::from_reason(err.to_string())`. When parity matters, the
   message matches what the JS path threw.
6. **Free native memory explicitly.** V8 cannot see native allocations, so
   stateful objects hold `Option<Inner>` and expose a `free()` the server calls
   when the session ends; every method treats `None` as freed.
7. **Numbers.** JS numbers become `u32`/`i64`/`f64` at the boundary; convert
   to `usize` and newtypes with `TryFrom` inside, never with `as` on
   untrusted values.
8. **`#[napi(object)]` structs are wire types.** Keep them flat and stable,
   document each field, and do not leak crate-internal types through them.
9. **Feature gates.** Anything depending on libghostty-vt sits behind
   `#[cfg(feature = "ghostty")]` so `build.mjs --no-ghostty` still builds.

## The server side

- `native-core.ts` loads the binary once. A switch that is on with a binary
  that is missing or fails to load stays on the JS path, logs the reason, and
  reports it on Settings › Experimental.
- One switch per feature; `VORN_CORE=native|js` overrides all of them.
  Feature-specific overrides (like `VORN_GIT`) win over the setting.
- The JS path is not deleted in the same change. It goes only after the native
  path has been the default for a release.
- Tests that need the binary check for `packages/core/vorn_core.node` and skip
  with a reason when it is absent.

## Checklist for a new export

- [ ] Logic and tests are in the crate; the adapter only converts.
- [ ] `catch_unwind` on sync exports, or async with `spawn_blocking`.
- [ ] Bounded work per call on Node's thread; heavy work is async.
- [ ] Errors mapped to readable messages; no `unwrap` on JS input.
- [ ] Native memory released by an explicit `free()` where state is held.
- [ ] Switch, fallback and its log line exist on the server side.
- [ ] Parity test feeds the same input to both paths.
