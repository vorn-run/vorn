---
name: vorn-wp-builder
description: Build a native core work package in packages/core, from its spec to a merged PR. Use when starting or continuing one, or any Rust change that adds a native path to vornd, vorn-app or the CLI.
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
2. Find the code being changed and every test that covers it (the crate's
   `tests/`, `crates/vornd/tests/`, `tests/*.test.ts`). Those tests become your
   acceptance suite.
3. Read the existing crates (`packages/core/crates/*`) and how the nearest
   finished one is wired into vornd or vorn-app. Reuse their shapes.

## 2. Design the crate API first

- New logic goes in a plain crate under `packages/core/crates/<name>`
  (package `vorn-<name>`), added to the workspace `members`. No tokio unless
  the logic itself is concurrent.
- Write the public API as types and signatures with `///` docs before the
  bodies: what it owns, what it borrows, what can fail (an error enum), what
  is `Send`.
- Keep the API in the vocabulary of the design notes (record, cursor, epoch,
  checkpoint).

## 3. Implement with tests

- Unit tests in the crate (`#[cfg(test)] mod tests`) for logic and edge cases;
  integration tests in `crates/<name>/tests/` for the public API only.
- Property tests (seeded) for anything with a round-trip or an invariant,
  especially the test ids that say "property". Keep the failing seeds as
  regression cases.
- Criterion benches in `crates/<name>/benches/` when the spec has a number.
- Apply the `rust-idioms` skill throughout.

## 4. Wiring

- vornd, vorn-app or the CLI calls the crate; keep that layer thin (types,
  concurrency, errors).
- New behaviour ships on by default, with no Settings › Experimental switch.
  Anything a user should choose is a first-class setting on its own page.

## 5. Validate locally (before every push)

```sh
cd packages/core
cargo fmt --all --check
cargo clippy --release --locked --workspace --all-targets -- -D warnings
cargo test --release --locked -p <the crates you touched>
cd ../..
# The full build. --no-ghostty builds vornd without its session engine, so it
# holds no screens; use it only for work that needs no Ghostty.
yarn build:core
yarn lint && yarn format:check && yarn typecheck

# Acceptance: the tests that cover the changed path.
yarn vitest run tests/<the files the spec names>

# When the spec has a number.
yarn bench
yarn bench --baseline=bench/baselines/<platform>-<arch>.json

# When the change touches a per-byte, per-frame or per-pane path (terminal
# output, vorn-app's frame, anything run per pane): see rust-idioms.
yarn bench:panes --panes 8,16
```

A test that starts vornd or vorn-sessiond gives it a temp HOME and data dir,
`VORN_EXIT_WITH=<the test's pid>` (it and the sessions it starts end with the
test run) and, for vornd, `VORN_TAILSCALE=none`. The vitest sandbox sets both.
CI fails when one outlives the suite:
`node scripts/check-leftover-processes.mjs <temp root> <checkout>`.

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

## 6. Commit, PR, review, merge

- No Claude attribution: plain commit messages with no `Co-Authored-By:` or
  `Claude-Session:` trailers, and no "Generated with Claude Code" line or
  session link in PR bodies or comments.
- Keep the PR to the package. Note anything out of scope as a follow-up.
- PR body: brief. What the PR changes and how it was tested (commands, test
  names, bench numbers). No attribution or "Requested
  by" lines, no roadmap or work-package numbers, no links to design notes or
  artifacts, and nothing about later phases or plans.
- Every PR waits for Copilot's review. Fix the worthwhile comments, reply to
  and resolve the rest, re-request review; at most two re-reviews per PR.
- Merge once Copilot's comments are resolved, your own re-read of the diff is
  done and CI is green. A stacked package stays draft until the one below merges,
  then retarget to `main`, finish and merge.
- Never cut a release; that waits for the maintainer's explicit yes.
