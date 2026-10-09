---
name: vorn-wp-builder
description: Build a native core work package in packages/core, from its spec to a merged PR. Use when starting or continuing one, or any Rust change that adds a native path behind a Settings › Experimental switch.
---

# Building a native core work package

Your brief gives the work package a spec with four fields. Treat them as the
contract:

| Field          | What it means for you                                                         |
| -------------- | ----------------------------------------------------------------------------- |
| **Read**       | Every file and design-note section listed. Read them all before writing code. |
| **Must pass**  | The tests that decide acceptance. Each test id it names is a test you write.  |
| **Depends on** | Check it is merged on `main`. If not, build stacked and keep the PR a draft.  |
| **Done when**  | The definition of done. Check every item before you open the PR for review.   |

Design notes define shared terms (records, cursors, offsets); use their names
in code.

## 1. Orient

1. Read the spec and every linked section. Note each must-pass test id.
2. Find the TypeScript being replaced (`packages/server/src/...`) and every
   test that covers it (`tests/*.test.ts`). Those tests become your acceptance
   suite with the switch on.
3. Read the existing crates (`packages/core/crates/*`) and the adapter for the
   nearest finished package (`src/git.rs` is the async model, `src/screen.rs` the
   stateful one). Reuse their shapes.

## 2. Design the crate API first

- New logic goes in a plain crate under `packages/core/crates/<name>`
  (package `vorn-<name>`), added to the workspace `members`. No napi, no
  tokio unless the logic itself is concurrent.
- Write the public API as types and signatures with `///` docs before the
  bodies: what it owns, what it borrows, what can fail (an error enum), what
  is `Send`.
- Keep the API in the vocabulary of the design notes (record, cursor, epoch,
  checkpoint), not of the JS implementation.

## 3. Implement with tests

- Unit tests in the crate (`#[cfg(test)] mod tests`) for logic and edge cases;
  integration tests in `crates/<name>/tests/` for the public API only.
- Property tests (seeded) for anything with a round-trip or an invariant,
  especially the test ids that say "property". Keep the failing seeds as
  regression cases.
- Criterion benches in `crates/<name>/benches/` when the spec has a number.
- Apply the `rust-idioms` skill throughout.

## 4. Adapter and switch

- Thin napi adapter in `packages/core/src/<name>.rs`, following the
  `napi-boundary` skill.
- The server side (`packages/server/src/native-core.ts` and the module being
  replaced) chooses the path by the switch. A switch that is on with a missing
  or failing binary falls back to the TypeScript, logs why, and says so on the
  settings page.
- One switch in Settings › Experimental for the whole batch of native work
  being tried, never one per feature. Add to the batch's switch if it has
  one.

## 5. Parity

- Add or extend a parity test under `tests/` that feeds identical input to the
  JS and native paths and compares per feature. Accepted differences get a
  named normalizer in a `tests/helpers/*-parity.ts` file, never an inline
  tweak.
- Tests that need the binary skip cleanly when `packages/core/vorn_core.node`
  is absent, as `tests/js-reference.test.ts` does.
- When a batch becomes the default, record the TypeScript path's outputs as
  fixtures first (`tests/fixtures/js-reference/`), then remove its switch and
  the TypeScript in the same change.

## 6. Validate locally (before every push)

```sh
cd packages/core
cargo fmt --all --check
cargo clippy --release --locked --workspace --all-targets -- -D warnings
# The plain crates only, as CI does: vorn-core's own test binary needs symbols
# only Node provides and may not link. The adapter is covered from vitest.
cargo test --release --locked --workspace --exclude vorn-core
cd ../..
# The full build. --no-ghostty builds vornd without its session engine, so it
# holds no screens; use it only for work that needs no Ghostty.
yarn build:core
yarn lint && yarn format:check && yarn typecheck
# The whole CI pipeline, including the 80 % patch-coverage gate (diff-cover
# against origin/main). CI collects coverage from vitest shards that have no
# native binary, so switch-on branches need tests that fake the binary's side.
yarn ci:local

# Acceptance runs with the switch off and on: the tests that cover a switched
# path turn it on themselves, as the server would read it from the settings.
yarn vitest run tests/<the files the spec names> tests/native-core*.test.ts

# When the spec has a number.
yarn bench
yarn bench --baseline=bench/baselines/<platform>-<arch>.json
```

The toolchain is pinned in `packages/core/rust-toolchain.toml`; a new stable
lint cannot turn clippy red on its own, so a clippy failure is yours.

### Sandbox notes

- **Zig 0.16.0 exactly** for libghostty-vt:
  `pip install ziglang==0.16.0` and a `zig` shim that runs
  `python3 -m ziglang "$@"`, first on `PATH`.
- Zig's own HTTP client fails through the agent proxy. Prefetch each `.url` in
  Ghostty's `build.zig.zon` files with curl and `zig fetch <file>`; for
  GitHub archive tarballs, `git clone` at the commit, strip `.git`, and
  `zig fetch <dir>`.
- Yarn 4 via corepack may not download through the proxy. Use
  `npm pack @yarnpkg/cli-dist@<version>` and a `yarn` shim instead.
- The sandbox exports `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_*`/`GIT_CONFIG_VALUE_*`,
  which makes the `vorn-git` gix fast path decline. Unset them to exercise
  gix.
- Running as root makes one connector-packs chmod test fail; it passes in CI.
- If a step truly cannot run here (a macOS-only check, a device), say so in the
  PR rather than pushing to see what CI does.

## 7. Commit, PR, review, merge

- No Claude attribution: plain commit messages with no `Co-Authored-By:` or
  `Claude-Session:` trailers, and no "Generated with Claude Code" line or
  session link in PR bodies or comments.
- Keep the PR to the package. Note anything out of scope as a follow-up.
- PR body: brief. What the PR changes and how it was tested (commands, test
  names, bench numbers), and the switch name. No attribution or "Requested
  by" lines, no roadmap or work-package numbers, no links to design notes or
  artifacts, and nothing about later phases or plans.
- Every PR waits for Copilot's review. Fix the worthwhile comments, reply to
  and resolve the rest, re-request review; at most two re-reviews per PR.
- Merge once Copilot's comments are resolved, your own re-read of the diff is
  done and CI is green. A stacked package stays draft until the one below merges,
  then retarget to `main`, finish and merge.
- Never cut a release; that waits for the maintainer's explicit yes.
