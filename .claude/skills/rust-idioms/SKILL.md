---
name: rust-idioms
description: Idiomatic, production Rust for Vorn's crates (types, errors, memory, async, testing, performance, tooling, dependencies). Use while writing or reviewing any Rust in the repo, and as the checklist for re-reading your own diff before a push.
---

# Rust idioms for Vorn

Written for the pinned toolchain (`packages/core/rust-toolchain.toml`) and for
a terminal core where every byte a shell prints passes through this
code. When a rule here and the surrounding code disagree, match the surrounding
code and raise it in the PR.

## Types and data

- **Model states as enums**, not flags plus `Option`s. If two fields are only
  valid together, they belong in one variant.
- **Newtypes for units that must not mix**: byte offsets, record numbers,
  epochs, UTF-16 lengths, columns. `struct Rseq(u64)` costs nothing and makes a
  swapped argument a compile error.
- **Strings**: take `&str` (or `&[u8]` for terminal bytes, which are not
  UTF-8 guaranteed), return `String` only when the caller needs ownership.
  `Cow<'_, str>` when a function usually returns its input unchanged.
- **Collections**: `Vec` by default; `VecDeque` for rings; `HashMap` for
  lookups; `BTreeMap` when order matters or for deterministic output in tests
  and serialization. Derive `Hash, Eq, PartialEq` together, never by hand
  unless the semantics differ.
- **Conversions** through `From`/`TryFrom`, not ad hoc `to_x()` helpers. Use
  `TryFrom` whenever the conversion can lose information (`usize` to `u32` at
  the napi boundary is the usual one).
- Derive `Debug` on every public type; derive `Clone`, `Default`, `Copy` only
  when they are cheap and meaningful.

## Errors

- Library crates return `Result<T, CrateError>` with an error enum per crate
  (implement `std::error::Error` and `Display`, by hand or with `thiserror` if
  it is already in the tree). The message says what failed and with which
  input, in the words the TypeScript used when parity matters.
- `?` to propagate, with `From` impls between layers. No `Box<dyn Error>` in a
  library's public API.
- `unwrap()`/`expect()` only for invariants the code itself establishes, with
  `expect("why this cannot fail")`. Never on input from a PTY, a file, the
  network or JS.
- Panics never cross FFI; see the `napi-boundary` skill.

## Ownership and memory

- Prefer moving and borrowing to cloning. A `.clone()` on the hot path needs a
  reason; on a `String` or `Vec` per chunk it is a bug.
- Choose the smallest tool: `Box` for one heap value, `Rc`/`RefCell` only on a
  single thread, `Arc` to share across threads, `Arc<Mutex<_>>` or
  `Arc<RwLock<_>>` for shared mutation. Before adding a lock, try ownership by
  one thread plus a channel.
- Globals: `std::sync::LazyLock` / `OnceLock` (not `lazy_static`). No
  `static mut`.
- Preallocate when the size is known (`Vec::with_capacity`,
  `String::with_capacity`, `reserve`), and reuse buffers across calls instead
  of allocating per chunk. `extend_from_slice` / `copy_from_slice` for byte
  copies.
- `unsafe` only at an FFI edge, wrapped in a safe function, with a
  `// SAFETY:` comment stating the invariant. `#[repr(C)]` and `std::ffi`
  types for anything crossing a C boundary.

## Async and threads

- Async only where there is real concurrency (I/O, many sessions). A
  sequential computation is a plain function; call it from async code with
  `tokio::task::spawn_blocking`.
- Never block inside an async task: no `std::fs`, `std::process`, long loops
  or `Mutex` held across `.await`. Use `tokio::sync` primitives when a lock
  must span an await.
- Message passing over shared state: `tokio::sync::mpsc` (bounded, so a
  slow consumer applies backpressure), `oneshot` for replies, `broadcast` or
  `watch` for fan-out.
- Bound concurrency explicitly (a `Semaphore`, as `src/git.rs` does) rather
  than spawning per request.
- `tokio::select!` for cancellation and timeouts; every loop that waits has a
  way out.
- Avoid `block_on` inside a runtime. Bridging sync to async is a design smell
  here.
- Instrument long-lived tasks with `tracing` spans once a daemon exists.

## Testing

- Do not test what the type system proves. Test logic, boundaries and
  invariants: empty input, a chunk split mid escape sequence, mid UTF-8
  codepoint, a wide character at the last column, a record at an epoch change,
  a cursor exactly at a boundary.
- Unit tests next to the code; integration tests in `tests/` use only the
  public API, which also tests that the API is pleasant to use.
- Tests run in parallel. No shared mutable globals; give each test its own
  temp dir (`tempfile`) and its own instance.
- Property tests for round trips (serialize then restore, write then read,
  feed in one chunk versus many). Seed them and keep failing cases as
  fixtures.
- Fuzz (`cargo fuzz`) parsers of untrusted bytes when the spec calls for it;
  never let a fuzz input panic.
- `#[tokio::test]` for async code; prefer paused time (`start_paused = true`)
  to real sleeps.
- A test that needs something absent (the Ghostty build, git) skips with a
  clear reason; it never silently passes.

## Performance

- Measure in release (`--release`); debug numbers mean nothing. Benchmarks are
  Criterion benches in the plain crate, plus `yarn bench` for the end-to-end
  JS baseline.
- Do one pass over the bytes. `memchr` for scanning, slices instead of
  substrings, iterators over index loops (they elide bounds checks).
- Batch work across the FFI boundary; a call per chunk costs more than the
  work in it.
- Parallelize with `rayon` only for independent CPU-bound work on large
  inputs; the terminal path is usually latency-bound, not throughput-bound.
- No portable SIMD or nightly features; the toolchain is stable and pinned.

## Tooling and hygiene

- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must be clean.
  Fix lints; `#[allow(clippy::...)]` only with a comment saying why, on the
  smallest item.
- `//!` doc on every module and `///` on every public item, saying why as well
  as what. Doc examples compile; use them for small API examples.
- After adding a dependency locally, `cargo tree -p <crate>` shows what it
  pulls in; check that before committing. `cargo tree -i <crate>` shows who
  depends on a crate. `cargo expand` debugs a macro (napi-derive included).
- Dependencies: prefer the standard library and crates already in
  `Cargo.lock`. Turn off default features and enable only what is used.
  Optional heavy deps sit behind a cargo feature (as `ghostty` does).
- Patch or fork a dependency only as a last resort, upstream the fix, and get
  back on the released version quickly.
- `Cargo.lock` is committed; CI builds with `--locked`.

## Before you push: re-read the diff

1. Any `unwrap`, `expect`, `panic!`, indexing or `as` cast on outside input?
2. Any allocation or clone per chunk that could be a reused buffer?
3. Any blocking call reachable from an async task or from Node's thread?
4. Any `unsafe` without a `SAFETY` comment, or exposed without a safe wrapper?
5. Do new public items have docs, and do error messages say what failed?
6. Is every new test checking logic, and does each must-pass id have one?
