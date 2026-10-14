# Dead code report

Scan of the Vorn monorepo on `8f46b897` (release 0.8.0-beta.10). There is no top-level `src/` tree. TypeScript lives in `packages/*/src` (and is consumed by `tests/` and `scripts/`). Rust is `packages/core`. Swift is `apps/macos`.

High-confidence items below were removed on this branch. Medium and low items were left in place.

## How this was checked

- **TypeScript.** [Knip](https://knip.dev) 6.41.0 over the Yarn workspaces, plus the TypeScript language service (`getReferencesAtPosition`) on every exported function, class, const, and type in `packages/`. A symbol counted as unused only when that reference search and a repo-wide identifier search agreed. Knip's unused-file and unused-dependency lists were empty except for `shiki` (see Medium).
- **Rust.** `rustc 1.97.0` (the pin in `packages/core/rust-toolchain.toml`) with `cargo clippy -W dead_code` on the crates that build on Linux without Zig or a GPU: `vorn-logfile`, `vorn-size`, `vorn-term-proto`, `vorn-work`, `vorn-protocol`, `vorn-store`, `vorn-spawn`, `vorn-analysis`, `vorn-agents`. No private `dead_code` warnings. `rustc` does not flag `pub` items, so public items were checked by counting identifier occurrences across every `.rs` file under `packages/core/crates`, `packages/core/vendor`, and `apps/macos/grid-ffi`. `cargo udeps` was not run: it needs a nightly compiler and a full workspace build.
- **Swift.** No Swift toolchain on this Linux host, so this is static only. Every `private` / `fileprivate` declaration in `apps/macos` was checked for a second occurrence of its name in the same file (Swift `private` is file-scoped for top-level items, and type-private members are still named at the call site). None had a single hit. Swift sources are members of the local package in `apps/macos/Package.swift` by directory, so a file under `Sources/` or `Tests/` is part of a target even when `project.pbxproj` does not name it.
- **IPC.** Each string value of `IPC` in `packages/shared/src/types.ts` was searched as a substring across `packages/`, `apps/`, `tests/`, `scripts/`, and `docs/`, excluding `node_modules`, `target`, and `spikes`.

Clippy on any crate that links libghostty-vt stops in `packages/core/vendor/libghostty-vt-sys/build.rs`: Zig is not installed (`failed to execute zig build`). That blocks `vorn-screen`, `vorn-grid`, `vorn-engine`, `vorn-recovery`, `vornd`, and anything that depends on them (`vorn-app`, `vornui`). Those crates were reviewed with the identifier scan only.

## Removed (high confidence)

Nothing else in the repo names these. They are not inputs to `scripts/gen-store-schema.mjs`, not re-exported from a published package entry, and not cited by the Rust or Swift ports.

| What                      | Where                                         | Evidence                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `isShellSession`          | `packages/shared/src/types.ts`                | The only match in the repo was the definition. `isAiAgent` is the guard tests and callers use.                                                                                                                                                                                                                                                                            |
| `browserPartition`        | `packages/shared/src/types.ts`                | Electron `<webview>` partition helper. The function and the string `persist:vorn-browser-` occurred only there.                                                                                                                                                                                                                                                           |
| 63 `IPC` entries          | `packages/shared/src/types.ts`                | Channel string and key each occurred only inside the `IPC` object. The two wiring tests (`tests/extension-rpc.test.ts`, `tests/connector-pack-rpc.test.ts`) assert a different subset, which is still present.                                                                                                                                                            |
| `FIRE_GRACE_MS`           | `packages/core/crates/work/src/schedule.rs`   | `pub` constant, one occurrence. No reader in `Schedule::due` or elsewhere. The 10s grace it described is not implemented.                                                                                                                                                                                                                                                 |
| `LOOP_BODY_TOP`           | `packages/core/crates/vorn-app/src/canvas.rs` | `pub` constant, one occurrence. Call sites already use `LOOP_HEADER + LOOP_BODY_PAD`. The Swift layout keeps its own `loopBodyTop`.                                                                                                                                                                                                                                       |
| Re-exports from `pack.ts` | `packages/connector-sdk/src/pack.ts`          | `bundleDependencyFindings`, `bundledRequireFindings`, `lifecycleScriptFindings`, `readNearestPackageJson`, `MAX_PACK_BYTES`, `MAX_UNPACKED_BYTES`, and the `BundleRequest` type re-export. The package entry and the tests import those from `packaging.ts` / `index.ts`. `export type { BundleOutput }` stays: `tests/connector-pack.test.ts` imports it from `pack.ts`. |

The 63 removed channels are the Electron-era names (renderer, `<webview>`, dialogs, window chrome, the old updater). `WIDGET_FOCUS_TERMINAL` (`widget:focus-terminal`) was kept: `apps/macos/Sources/VornApp/Widget.swift` still contains that string.

`TERMINAL_FRAME`, `TERMINAL_RECONNECTED`, `TERMINAL_RENAME`, `TERMINAL_REORDER`, `TERMINAL_SET_GROUP`, `DIALOG_OPEN_DIRECTORY`, `DIALOG_OPEN_FILE`, `DIALOG_OPEN_IMAGE`, `DIALOG_SAVE_TEXT_FILE`, `VORND_STATUS`, `VORND_SESSION_HOLDERS`, `VORND_END_SESSION_HOLDER`, `SCHEDULER_STOP_RUN`, `BROWSER_ARTIFACT_SELECTION`, `BROWSER_ARTIFACT_PAINT`, `BROWSER_ARTIFACT_REVEAL`, `BROWSER_ARTIFACT_CLEAR`, `BROWSER_ATTACH`, `BROWSER_DETACH`, `BROWSER_WATCH_FILE`, `BROWSER_FILE_CHANGED`, `BROWSER_READ_MANIFEST`, `BROWSER_SET_TWEAK`, `BROWSER_ARTBOARD_ATTACH`, `BROWSER_ARTBOARD_DETACH`, `BROWSER_ARTBOARD_TWEAKS`, `BROWSER_ARTBOARD_POINT`, `BROWSER_PICK_START`, `BROWSER_PICK_CANCEL`, `BROWSER_PICKED`, `BROWSER_ANNOTATE`, `BROWSER_TAB_COMMAND`, `BROWSER_TABS_CHANGED`, `WINDOW_MINIMIZE`, `WINDOW_MAXIMIZE`, `WINDOW_CLOSE`, `WINDOW_IS_MAXIMIZED`, `WINDOW_MAXIMIZED_CHANGED`, `WIDGET_HIDE`, `WIDGET_TOGGLE`, `WIDGET_RENDERER_STATUS`, `WIDGET_SET_ENABLED`, `WIDGET_PERMISSION_RESPONSE`, `DEVICE_SCREEN_INFO`, `DEVICE_VIDEO_START`, `DEVICE_VIDEO_STOP`, `DEVICE_PICKED`, `DEVICE_ANNOTATE`, `UPDATE_INSTALL`, `UPDATE_SET_CHANNEL`, `UPDATE_STATUS`, `UPDATE_CHECK`, `UPDATE_DOWNLOAD`, `UPDATE_SET_AUTO_DOWNLOAD`, `UPDATE_GET_STATUS`, `SERVER_RUNTIME_STATUS`, `SERVER_GET_RUNTIME_STATUS`, `SERVER_UPGRADE`, `OPEN_EXTERNAL`, `CLI_STATUS`, `CLI_INSTALL`, `CONNECTION_SIGN_IN`, `CONNECTION_SIGN_OUT`.

## Medium — needs a person

These look unused from TypeScript, and many have no in-repo caller. They were not removed. The usual reason is that Rust or Swift still names them as the behavior that was ported, or the value is a wire contract that native code has inlined.

### Reference helpers in `@vornrun/shared` (private package)

Language service: no TypeScript reference outside the definition. The same name appears in Rust and/or Swift comments or as a parallel function. Deleting them drops the executable spec those comments point at.

| Symbol                                                                                                                                              | File                                    | Also named in                                                                                                                                                                    |
| --------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `headMoved`                                                                                                                                         | `packages/shared/src/types.ts`          | `apps/macos/Sources/VornCore/Models.swift`, `SessionChrome.swift`                                                                                                                |
| `DEFAULT_WORKSPACE`                                                                                                                                 | `packages/shared/src/types.ts`          | `packages/core/crates/vorn-app/src/client/store.rs`                                                                                                                              |
| `getProjectHostIds`, `getProjectRemoteHostId`                                                                                                       | `packages/shared/src/types.ts`          | `NodeFormsData.swift` (remote host), `CommandPalette.swift`, `palette.rs`, `worktrees/src/scan.rs`. The remote-host helper is the only TypeScript caller of `getProjectHostIds`. |
| `actionInputField`                                                                                                                                  | `packages/shared/src/types.ts`          | `NodeFormsAPI.swift`, `connectors/src/connections.rs`                                                                                                                            |
| `isImplicitConnection`                                                                                                                              | `packages/shared/src/types.ts`          | `ConnectorModels.swift`, `vorn-app/src/plugins.rs`                                                                                                                               |
| `borrowableFromManifest`                                                                                                                            | `packages/shared/src/types.ts`          | `ExtensionCopy.swift`, `vorn-app/src/plugins.rs`                                                                                                                                 |
| `workflowRunId`                                                                                                                                     | `packages/shared/src/types.ts`          | `store/src/runs.rs`                                                                                                                                                              |
| `DEFAULT_ARTIFACT_DIRS`, `DEFAULT_IDLE_DAYS_THRESHOLD`                                                                                              | `packages/shared/src/types.ts`          | `worktrees/src/lib.rs`, `worktrees/src/scan.rs`                                                                                                                                  |
| `connectionPartition`                                                                                                                               | `packages/shared/src/types.ts`          | Only caller of `CONNECTION_PROFILE_PREFIX`, which `vorn-app/src/signin.rs` documents. The partition function itself has no caller.                                               |
| `supportsModelSelection`, `validateModelId`                                                                                                         | `packages/shared/src/agent-models.ts`   | `StepEdits.swift`, `agents/src/launch/model.rs`, `vorn-app/src/models.rs`. The types in this file are still imported by `protocol.ts`.                                           |
| `slugify`, `ensureUniqueSlug`, `CONTEXT_REF`, `isContextRef`, `getAvailableContextVars`, `buildInputVars`, `containsContextRef`, `getAncestorNodes` | `packages/shared/src/template-vars.ts`  | Swift `TemplateVars.swift` / `RunInputs.swift` and `vorn-app/src/vars.rs`. A different `slugify` in `workflow-portability.ts` is the one TypeScript still calls.                 |
| `DEFAULT_STEP_TIMEOUT_MINUTES`, `hasFailedStep`, `isRunStoppable`, `getWorktreeMode`, `schedulerExecutionContext`                                   | `packages/shared/src/workflow-graph.ts` | `work/src/graph.rs`, `vorn-app/src/runs.rs`, Swift workflow views. Neighbors such as `failedStep` and `buildGraph` are used in the same file, so they are not dead on their own. |

### Named contracts with no TypeScript reader

The identifier occurs only in `packages/shared`. The value is duplicated as a literal in Rust (and sometimes tests).

| Symbol                     | Value                         | Native copy                                                               |
| -------------------------- | ----------------------------- | ------------------------------------------------------------------------- |
| `CONNECTOR_POLL_EVENT`     | `'mcpPoll'`                   | `connectors/src/connections.rs` `POLL_EVENT`                              |
| `CONNECTOR_AUTH_RUNGS`     | the five rungs                | The `ConnectorAuthRung` union is the live type. The array has no reader.  |
| `ARTIFACT_KINDS`           | `'page' \| 'doc' \| 'design'` | Mirrors the `ArtifactKind` union. No reader.                              |
| `RUNTIME_PROTOCOL_VERSION` | `1`                           | Handshake field, no import of the const.                                  |
| `RPC_NOT_AUTHENTICATED`    | `-32001`                      | `vornd/src/serve/grid_ws.rs`, `socket.rs`                                 |
| `DEFAULT_SERVER_PORT`      | `50091`                       | `vornd/src/serve/port.rs` `DEFAULT_PORT`                                  |
| `SERVER_PORT_ENV_VAR`      | `'VORN_SERVER_PORT'`          | Not imported. The string was not found elsewhere under that const's name. |
| `ENDPOINT_FILENAME`        | `'vorn.sock'`                 | `apps/macos/grid-ffi/src/lib.rs` uses the filename in a test path.        |
| `SERVER_LOG_FILENAME`      | `'server.log'`                | Named from `cli/src/client.rs`.                                           |

`WIDGET_FOCUS_TERMINAL` is the one IPC entry whose string still appears outside `types.ts` (`Widget.swift`) while nothing reads the TypeScript key.

### Exports that are only used in their own file

Knip reports these as unused exports. The language service shows in-file references, and they are not part of `packages/connector-sdk/src/index.ts` or `packages/mcp/src/index.ts`. Un-exporting them is safe only after confirming no out-of-repo import of the source path. They are not dead code.

- `packages/connector-sdk/src/loopback.ts`: `LOOPBACK_HOSTS`
- `packages/connector-sdk/src/define.ts`: `EXTENSION_PLATFORMS`, `hasNestedQuantifier`
- `packages/connector-sdk/src/request.ts`: `sendRequest` (also `resolveTemplates`, `asOutput`, `MAX_REQUEST_PAGES` are re-exported from the package entry, which is the public API)
- `packages/connector-sdk/src/check.ts`: `liveExamines`
- `packages/mcp/src/relay.ts`: `BACKOFF`
- `packages/mcp/src/tools/browser.ts`: `toTarget`
- `packages/mcp/src/tools/workflows.ts`: `describeConfigIssues`, `validateLoopStructures`, `approvalNode`, `askedBy` are used in file. `nodeShapeSchema` is also loaded by name from `scripts/gen-mcp-tools.mjs` (`workflows.nodeShapeSchema`), which Knip does not see.
- `packages/mcp/src/tools/describe-nodes.ts`: `WORKFLOW_NODES_REFERENCE` is used in file and passed through `scripts/gen-mcp-tools.mjs`. Knip false positive.

Several `packages/shared` exports (`getProjectHostIds`, `SDK_FILTER_KEYS`, `MINIMIZED_PLACEMENTS`, `boundConnectionKey`, `TEMPLATE_VARIABLES`, `DEFAULT_GATE_ROUNDS`, and others) are the same shape: used in the defining file, exported because the file is a package entry. Knip `--include-entry-exports` lists them. Treat that list as "no in-repo importer," not as dead code.

### `shiki` on the root `package.json`

Knip flags `shiki` (dependency, line 42). Nothing imports the specifier `'shiki'`. `apps/macos/scripts/shiki/entry.mjs` imports `@shikijs/*`, which this repo only resolves because `shiki` depends on them. Removing the dependency without adding those packages directly would break the highlighter bundle. Not removed.

### Rust public items with one occurrence

| Item                                                                                                  | File                                         | Why it stayed                                                                                                                   |
| ----------------------------------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `caps::IMAGES`                                                                                        | `packages/core/crates/term-proto/src/msg.rs` | Reserved Kitty-graphics capability bit (`1 << 63`). Deliberately not included in `caps::ALL`.                                   |
| `line::SOFT`, `text::PX9`, `text::PX10`, `text::PX11`, `text::PX13`, `text::MEDIUM`, `text::SEMIBOLD` | `packages/core/crates/vornui/src/theme.rs`   | Design-scale tokens. Neighbors in the same modules are used. Unused tokens may be the scale the screens are meant to pick from. |

`#[allow(dead_code)]` in the tree is either `cfg`-gated (`boot.rs` `btime` / `boottime`, `session.rs` `me` on Windows, `app.rs` `Picked` on wasm) or a blanket allow on test support modules (`*/tests/common/mod.rs`, `engine/tests/gridrig`, `vorn-app/tests/support`). Those helpers are unused in some configurations and used in others. Not removed.

### Feature flags

| Flag                         | Crate               | Status                                                                                                                                                                          |
| ---------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `engine`                     | `vornd`             | Default. `packages/core/scripts/build.mjs --no-ghostty` turns default features off. Live.                                                                                       |
| `webview`                    | `vorn-app`          | Default. `vorn-web` and `vorn-cli` depend on `vorn-app` with `default-features = false`. Live.                                                                                  |
| `serde`                      | `term-proto`        | Enabled by `sessiond`, `recovery`, `sessiond-wire`. Live.                                                                                                                       |
| `tokio`                      | `spawn`             | Enabled by `vornd` and `connectors`. Live.                                                                                                                                      |
| `test-support`               | `connectors`        | Enabled by `extensions`. Gates `connectors/src/pack.rs`. Live.                                                                                                                  |
| `kitty-graphics`             | `libghostty-vt-sys` | Empty feature. No `cfg(feature = "kitty-graphics")` anywhere. Never enabled.                                                                                                    |
| `pkg-config`, `link-dynamic` | `libghostty-vt-sys` | Implemented in `vendor/libghostty-vt-sys/build.rs`. Nothing in the workspace turns them on. They are the supported ways to link a system Ghostty instead of the vendored build. |

### Published package surface

`@vornrun/connector-sdk` and `@vornrun/mcp` are published (`private` is not set). Knip `--include-entry-exports` reports a large set of entry-file exports with no importer inside this repo (protocol types, harness helpers, `EXTENSION_PERMISSIONS`, and so on). Those are the SDK. They were not treated as dead.

`packages/shared/src/store-types.ts` has no TypeScript importer. `scripts/gen-store-schema.mjs` reads it as a schema source and `vorn-protocol` turns that schema into Rust. Same for the record types in `types.ts` listed in that script. Not dead.

## Low

- `unreachable!()` in `vornui/src/ui.rs`, `recovery/src/gen.rs`, `screen/src/lib.rs`, `mcp/src/tools/connectors.rs`, `vorn-app/src/tasks.rs`, `analysis/src/lib.rs` (and two test-only sites). Each is an invariant the surrounding match already established, not an unreachable branch a caller can hit.
- Generated FFI constants in `packages/core/vendor/libghostty-vt-sys/src/bindings.rs`. Hundreds of `pub const` names occur once. They are the C API surface of the bindings, not hand-written dead code.
- No unused TypeScript files. Knip reported none, and ESLint's `no-unused-vars` is clean on the product sources (the only warnings were temporary scanners under `.scratch/`, which are not part of the branch).
- No unused private Swift declarations under the file-local name check. Protocol conformances, `#selector`, and stringly keys would not show up this way; none of the private names were single-hit, so that gap did not produce a removal candidate.

## Spikes and benches

Left untouched.

**`spikes/vornui/`** — GPUI vs vornui prototype (`REPORT.md`). The report's recommendation is already the crate at `packages/core/crates/vornui`. The spike is a finished experiment, not a live dependency. Deleting it is a history decision, not a dead-code one. Not clearly safe to remove without that call.

**`packages/core/bench/`** — `bench/scale` is a workspace member and has result directories dated 2026-10-14 (`results/2026-10-14-5cc244d0/`). `bench/many-panes` has `darwin-arm64-2026-10-14.json`. Not stale.

## What still needs a person

1. The shared TypeScript helpers in the medium table. Native code has its own copies and comments that name the TypeScript. Decide whether `@vornrun/shared` should keep those functions as the spec or whether the Rust/Swift versions are now the source of truth.
2. Wire constants (`DEFAULT_SERVER_PORT`, `RPC_NOT_AUTHENTICATED`, `ENDPOINT_FILENAME`, and the others above). They document strings the native code has inlined. Keeping them avoids a third copy drifting; they currently have no TypeScript reader.
3. `shiki` vs declaring `@shikijs/*` directly for `apps/macos/scripts/shiki/entry.mjs`.
4. Empty `kitty-graphics` feature, and whether `pkg-config` / `link-dynamic` should stay on the vendored Ghostty crate for out-of-tree builds.
5. Unused vornui type-scale tokens (`SOFT`, `PX9`–`PX13`, `MEDIUM`, `SEMIBOLD`).
6. Whether `spikes/vornui` should be deleted now that vornui has landed.
7. `FIRE_GRACE_MS` was removed because nothing read it. If a 10 second schedule grace is a rule the server should enforce, that belongs in `Schedule::due`, not as an unused constant.
