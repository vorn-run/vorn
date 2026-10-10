# Vorn for macOS

The native app, as a Swift package (`swift build`, or open `Package.swift` in Xcode). macOS 14 first; iPadOS 17 builds from the same modules.

| Module             | Holds                                                                                              |
| ------------------ | -------------------------------------------------------------------------------------------------- |
| `VornCore`         | The vornd client: endpoint and credential, JSON-RPC over `/ws`, subscriptions, models and services |
| `VornUI`           | Theme tokens from `src/renderer/theme.css`, Lucide glyphs, agent and project icons                 |
| `VornTasks`        | The tasks board: list and kanban, cards, detail panel, new task dialog, view options               |
| `VornTasksPreview` | A host that shows `TasksView` alone, for development and screenshots                               |

## Mounting the tasks board

```swift
let store = TasksStore(service: client)          // any TaskService; VornClient is one
store.scope = .project("vorn")                   // or .all, .projects([...])
store.openSession = { task in ... }              // nil hides the session actions
store.isSessionLive = { task in ... }
store.onToast = { message, kind in ... }

TasksView(store: store)                          // the board, detail panel, dialog and popovers
TaskViewOptionsButton(store: store)              // goes in the top bar; ⌘J
```

`TasksView` loads the board on appear and reloads it whenever vornd reports `config:changed`.

## Preview host

```sh
swift run VornTasksPreview --sample                                  # a board held in memory
swift run VornTasksPreview --data-dir DIR                            # a vornd serving DIR
swift run VornTasksPreview --data-dir DIR --seed                     # fill a test vornd with the sample board
swift run VornTasksPreview --data-dir DIR --render out.png --state kanban --size 1200x700
```

`--state` is one of `list`, `kanban`, `dialog`, `detail` or `options`. Pointed at the app's own `~/.vorn`, the host connects read-only and the client refuses every write.

A test vornd for development:

```sh
packages/core/target/debug/vornd --data-dir apps/macos/.build/test-vornd --port 50391 --host 127.0.0.1
```

## Tests

`swift test` runs the model and store tests, and drives a vornd on a temporary data directory when `packages/core/target/debug/vornd` (or `$VORND_BIN`) exists.
