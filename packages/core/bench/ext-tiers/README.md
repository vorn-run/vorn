# ext-tiers: one extension, three ways to run it

E9.4. Worktree cleanup and the Widget, run as Vorn runs a built-in (compiled
into vornd), as a program (a process speaking JSON-RPC over stdio) and as a
Wasm component (WASI preview 2, in Wasmtime inside vornd), measured side by
side. Each tier uses the built-in's own manifest and templates; only the
`[service]` line differs (`command = [...]` or `component = "ext.wasm"`).

```sh
yarn bench:ext-tiers            # builds (release), runs, writes results/
yarn bench:ext-tiers --runs 3   # fewer runs
```

It writes `results/results.json`, `results/summary.md` (the tables, as
medians of the runs) and `fixtures/widget-asking.json`. The fixture holds the
rows the Widget component published; vorn-app's templates test draws them on
every surface.

| crate | what it is |
|---|---|
| `logic` (`ext-tiers-logic`) | Both extensions written once, synchronously, against a small `Host`: ported from `crates/vornd/src/native/extensions/builtin`, with their tests |
| `program` (`ext-tiers-program`) | That logic behind stdio JSON-RPC |
| `component` (`ext-tiers-component`) | That logic behind `crates/extensions/wit/extension.wit` (wit-bindgen), built for `wasm32-wasip2` |
| `probe` (`ext-tiers-probe`) | A host that starts services and does nothing else. It is built without the Wasm tier, with its runtime and with its compiler, to weigh each |
| `.` (`vorn-bench-ext-tiers`) | The harness. The built-in tier runs the logic in process, with calls through the same gate. The others run through vornd's router, lifecycle and gate |

Each measurement runs in a process of its own. The program and component
each serve both extensions, chosen by the id in `guest.init`. `bench.*`
actions (echo, crash, spin, grow) are answered by the glue, for the bench only.

What it measures:

- **Start:**
  - Cold start: first call, restarts, and the first publish after an event.
  - Call round trips: p50 and p99.
- **Throughput:** sustained events/s with 32 terminals, and CPU per event:
  - `session.status` storms;
  - `agent.permission` bursts;
  - `worktree.idle` bursts.
- **Memory:** idle and busy. A program's is its own footprint plus vornd's delta. A component's is vornd's delta, which includes the component's linear memory.
- **Crashes:** an endless loop, a 256 MB grab, panics until the crash budget holds the extension, and whether the host still answers.
- **Rows:** whether every tier leaves the same rows, in the same order, at every step of a scripted session.
- **Sizes and lines of code.**
