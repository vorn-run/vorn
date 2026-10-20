---
name: vorn-extension-author
description: Write, change or review a Vorn extension (a program, a Wasm component or one-shot commands), including the ones Vorn ships in packages/core/extensions. Use for anything touching a vorn-extension.toml, the extension API's version, or what an extension may call.
---

# Writing a Vorn extension

Read `docs/extensions.md` first, then `docs/wasm-extensions.md` for
components and the element tree. The API is defined once, in
`packages/core/crates/extensions/wit/extension.wit`.

## The rules

1. **One API, `vorn:extension@1.0`.** Never add another api id. Every
   manifest sets `api = "vorn:extension@1.0"`; `vorn = ">=1.0"` is optional
   and is a range of API versions, not of Vorn releases.
2. **Public only.** An extension, Vorn's own included, calls only the
   methods in `CORE_METHODS` (`gate.rs`) and listens only to `CORE_EVENTS`
   (`events.rs`), each under the permission its manifest declares. If it
   needs something the API cannot do, add it to the API (WIT, `gate.rs`,
   docs), never a private path.
3. **A built-in builds on `vorn-extension-sdk` alone.** No other path
   dependency, no `vorn_extension_sdk::bindings` or `::guest`. An exception
   goes in `packages/core/freeze.toml` with a one-line reason.
4. **The core stays frozen.** A new place where a core or host crate names a
   built-in (its id, its crate or its vornd module) fails
   `cargo test -p vorn-extensions --test freeze` until it is listed in
   `freeze.toml` under `[exceptions.hooks]` with a reason. Prefer moving the
   behaviour behind the public API.
5. **Versioning.** A minor version only adds. Changing or removing anything
   is a major: bump the WIT package, `wit/stdio.json`
   (`VORN_UPDATE_WIT=1 cargo test -p vorn-extensions publishes_the_mapping`)
   and every manifest, and say so in the PR.
6. **Handshake.** A program checks `guest.init`'s `ctx.api` and refuses a
   Vorn older than it was built for with a message naming both versions; a
   component is checked by vornd before linking.

## Checks

```sh
cargo test --profile ci -p vorn-extensions --test freeze
cargo test --profile ci -p vorn-extensions --lib compat
```

A manifest error names its field and line; keep it that way in any new
field you add to `manifest/file.rs`.
