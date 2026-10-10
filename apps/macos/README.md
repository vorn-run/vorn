# Vorn for macOS

The native app, as a Swift package (`swift build -c release`, or `Vorn.xcodeproj`, generated from `project.yml`). It talks to vornd over JSON-RPC and draws terminals with the grid in `grid-ffi` (build it first with `grid-ffi/build.sh`).

| Module             | Holds                                                                                               |
| ------------------ | --------------------------------------------------------------------------------------------------- |
| `VornCore`         | The vornd client: endpoint and credential, JSON-RPC over `/ws`, subscriptions, models and services  |
| `VornUI`           | Theme tokens from `src/renderer/theme.css`, Lucide glyphs, agent and project icons, shared controls |
| `VornTerminals`    | Terminal panes over vornd's grid socket                                                             |
| `VornTasks`        | The tasks board: list and kanban, cards, detail panel, new task dialog, view options                |
| `VornWorkflows`    | Workflows: runs list and detail, gates, the workflow page                                           |
| `VornApp`          | The shell: sidebar, top bar, and the sessions, tasks and workflows views                            |
| `VornTasksPreview` | A host that shows `TasksView` alone, for development and screenshots                                |
| `WorkflowsPreview` | A host that shows `WorkflowsView` alone, from fixtures or a vornd                                   |

## Offscreen renders

Nothing opens a window. Each host draws to a PNG at 2x:

```sh
.build/release/Vorn --render out.png [--view sessions|tasks|workflows] [--project NAME] [--no-sidebar]
.build/release/VornTasksPreview --sample --render out.png --state list|kanban|dialog|detail|options
.build/release/WorkflowsPreview --fixtures Tests/VornWorkflowsTests/Fixtures --render out.png --scene landing
```

`scripts/render-all.sh OUT` starts a throwaway vornd (temporary HOME and data dir), seeds it, and renders every screen into `OUT`. Pointed at the app's own `~/.vorn`, the hosts connect read-only and the client refuses every write.

## Tests

```sh
swift build -c release --build-tests -Xswiftc -enable-testing
VORND_BIN=../../packages/core/target/ci/vornd swift test -c release -Xswiftc -enable-testing --skip-build
```

With `VORND_BIN` (and `vorn-sessiond` beside it, or `VORN_SESSIOND_BIN`), `VornE2ETests` starts its own vornd on a temporary HOME and data dir, types into a shell and reads the echo, adds and moves tasks, and lists a workflow and its runs. `VORN_E2E_REQUIRED=1` turns a missing binary into a failure.
