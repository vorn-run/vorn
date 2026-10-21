# Wasm extensions

An extension's service runs in one of two tiers, behind the same manifest,
the same API and the same permissions:

- **A program**: any language, started by Vorn and spoken to over stdio, one
  JSON-RPC message per line. Fit for an extension that runs processes, reads
  files or uses a network SDK or a Node package. Every connector is one.
- **A Wasm component**: Rust (or anything that builds a component for
  `wasm32-wasip2`), run sandboxed inside vornd. It reaches nothing of the
  machine: no files, no network, no environment. It asks Vorn for what it
  needs through the API's imports, and Vorn holds each call to what the
  manifest declares. Fit for logic that reacts to events, keeps its own data
  and draws a UI. Worktree cleanup and the Widget, which Vorn ships, are two.

Both are written against `wit/extension.wit` in
`packages/core/crates/extensions`, the only definition of the API.

## A component's manifest

Only `[service]` differs from a program's: `component` names the `.wasm`
inside the package, in place of `command`.

```toml
id = "acme.hello"
name = "Hello"
version = "0.1.0"
api = "vorn:extension@1.0"

[build]
command = ["cargo", "build", "--release", "--target", "wasm32-wasip2"]

[service]
component = "target/wasm32-wasip2/release/acme_hello.wasm"
events = ["session.status"]

[declares]
uses = ["sessions.read", "storage"]

[[contributes.settings]]
id = "page"
title = "Hello"
icon = "puzzle"
tree = true
```

`[build]` runs when the extension is installed, and when `vorn ext link`
links it. The component is then compiled for this machine into Vorn's cache
by the `vorn` command; vornd itself holds no compiler and runs only what is
in that cache. A `.cwasm` shipped inside a package is never read.

## Start one

```sh
rustup target add wasm32-wasip2
vorn ext new acme.hello --wasm
cd acme.hello
cargo test                 # its logic, against a fake Vorn
vorn ext link --watch      # build, link, then rebuild and reload on every save
```

`vorn ext new` without `--wasm` writes the same extension as a Node program.

## The Rust SDK

`vorn-extension-sdk` (`packages/core/crates/extension-sdk`) is the API in
Rust. An extension is a type that implements `Extension`, and `export!`
makes it the component's guest:

```rust
use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{export, json, Error, Extension, Host, Init, Op, Value};

struct Hello;

impl Extension for Hello {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Hello)
    }

    fn render(&mut self, host: &mut dyn Host, _: &str, _: Surface) -> Result<tree::Node, Error> {
        let said = host.rows("hellos")?.len();
        Ok(col([button("hello", "Say hello"), stat(said.to_string(), "Hellos said")]))
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
        let n = host.rows("hellos")?.len() + 1;
        host.publish("hellos", vec![Op::Upsert(json!({ "id": n.to_string() }))]);
        Ok(json!({ "toast": format!("{} clicked", e.id) }))
    }
}

export!(Hello);
```

`Extension` has five methods, each with a default but `init`:

| method      | called for                                                                             |
| ----------- | -------------------------------------------------------------------------------------- |
| `init`      | the start of its instance                                                              |
| `handle`    | an action: a client, a menu item's `call`, an MCP tool (`mcp.<name>`), a workflow node |
| `on_events` | a batch of the events `[service] events` subscribes to                                 |
| `render`    | the tree of a `tree = true` contribution: `ui.render`                                  |
| `event`     | a click, a submit or a choice on that tree: `ui.event`                                 |

Every call back into Vorn goes through the `Host` it is handed:

| `Host`                                     | API                                                | permission           |
| ------------------------------------------ | -------------------------------------------------- | -------------------- |
| `call(method, args)`                       | `host.call`: a core method such as `sessions.list` | the method's own     |
| `publish(collection, ops)`                 | `host.publish`: rows its trees draw                | none                 |
| `rows(collection)`                         | `host.rows`: the rows it published                 | none                 |
| `start_session(req)`                       | `host.start-session`                               | `sessions.launch`    |
| `fetch(req, connection)`                   | `net.fetch`: HTTPS to a declared host              | `[declares] network` |
| `prefix`, `query`, `exec`, `get`, `set`    | `storage.*`: its own tables and values             | `storage`            |
| `transaction(statements)`                  | `storage.transaction`                              | `storage`            |
| `put_file`, `file`, `files`, `delete_file` | `blobs.*`: its own files                           | `storage`            |
| `log(level, message)`                      | `host.log`                                         | none                 |

`vorn_extension_sdk::testing::FakeHost` is a `Host` in memory: core methods
answered by closures, rows kept as Vorn keeps them, values and files in
maps. `dispatch` drives an extension the way the component does, so a test
covers `ui.render` and `ui.event` too:

```rust
let mut host = FakeHost::default().answer("sessions.list", |_| Ok(json!([])));
let mut hello = Hello::init(&Init::default(), &mut host).unwrap();
let click = json!({ "point": "page", "event": { "id": "hello", "kind": "click" } });
let done = dispatch(&mut hello, &mut host, tree::EVENT, click, &Ctx::default()).unwrap();
```

Vorn stops an instance that sits idle and starts a fresh one on its next
call, so keep what a person sees in published rows, storage or files, not
only in `self`. Worktree cleanup keeps the worktree its dialog asks about in
its `asked` row for that reason.

## UI: the element tree

An extension's UI is an element tree (`vorn-tree`), built with `tree::sdk`:
boxes, text, icons, status dots, buttons, toggles, fields, choices, badges,
notices, stats, meters, sections, toolbars, lists with grouping and
selection, boards, and the parts a pane draws (below). A program answers the same tree as JSON. The host
draws it on every client: the desktop, the web client, a terminal's cells
(`surface.cells`) and the phone.

Any contribution that draws says `tree = true`: a main view (`views`), a
sidebar section, a card's badges and footer, a sidebar row's marks
(`decorations`), a pane, the composer's strip, a shell's commands (`shell`),
a card of its own (`cards`), a settings page, a window and a dialog. Each is asked for
its tree with `ui.render` (`{point, surface}` in, `{tokens, tree}` out),
`point` being the contribution's id, so a tree contribution's id is unique
in its manifest. The host asks again whenever the extension's rows change,
and when the place shows.

A settings page is listed under Extensions in the Settings sidebar, and in
the palette as its title and "Settings". It may name the app's own page it
follows with `after` (`appearance`, `general`, `updates`, `notifications`,
`agents`, `mcp`, `keys` or `network`), as SSH & Hosts
follows `agents`, so it keeps that place and its palette entry, its bare
title.

A badge, a card footer and a sidebar row's marks draw once per session: the
tree's root is a box holding one child per session, its id the session's
id, and each card draws its own. `per_session` builds it; a session with no
child draws nothing. A sidebar section whose tree is an empty box does not
show.

A pane, a shell's commands and a card draw once for each place they show
in, so their tree is asked for with that place as `surface.scope`: the
session a pane or a shell's commands draw for, the row a card draws for. Its
events carry the same `scope`, and the `width` its place drew at. A tree with
a scope is asked for again when a row whose `id` is that scope changes, in
any of the extension's collections.

- `[[contributes.cards]]` names `rows`: each row of that collection is a
  card in the grid, after the session its `session` field names, called by
  its `title`.
- `[[contributes.shell]]`: a shell's commands, drawn over its live rows as
  blocks. Its tree holds a `log`.

A click, a toggle, a choice, a field's Return or a part's own event comes back as `ui.event`
(`{point, event}` in). Its answer, every field optional, says what follows:

| field    | does                                                                               |
| -------- | ---------------------------------------------------------------------------------- |
| `toast`  | a line that says what happened                                                     |
| `error`  | a line that says what failed                                                       |
| `patch`  | nodes changed in place, without a render                                           |
| `open`   | opens one of its dialogs, by id                                                    |
| `window` | opens one of its windows by id, in place of the one open                           |
| `close`  | `true` closes the dialog or window the tree is in                                  |
| `focus`  | a session to show in the main window, which comes forward                          |
| `show`   | `true` brings the main window forward                                              |
| `url`    | an `https:` page to open in the browser                                            |
| `signin` | opens Vorn's sign-in window: the target a `signin` method answers, and `id`        |
| `ask`    | `{title, ok?, event}`: Vorn's own alert asks first; OK tells the extension `event` |
| `same`   | `true`: nothing changed that the tree draws, so it is not asked for again          |

### The parts a pane draws

These hold data rather than children, and the client keeps what changes with
every key or pointer move (hover, typing, a drag) to itself:

| part      | holds                                                                                  | reports                                                                                       |
| --------- | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `tabs`    | tabs (label, file, preview, dirty, active, `icon`), `lead`, `tools` and `more` tools   | `select`, `open` (double click) and `close` a tab; `click` a tool                             |
| `outline` | rows a tree of folders deep, the one chosen, a `filter`, an `action`                   | `select` a row, `open` a leaf, `change` the filter, `click` the action with the row's id      |
| `split`   | one child or two, side by side, `ratio` to the first; under `narrow` px only the first | `resize` with the ratio dragged to                                                            |
| `code`    | a file's `text` on disk, a `draft` to start from, `rev`, `state`, `conflict`, `error`  | `change` and `submit` (save) with the text; `click` with `discard`, `force`, `disk` or `keep` |
| `log`     | a shell's commands, one per prompt, oldest first                                       |                                                                                               |
| `web`     | a web view: its `view` (the session it shows beside), the `url` asked for, a `notice`  | `page`, a JSON object in the value: what the page or its controls did (below)                 |
| `video`   | a device's screen: its `view` (session), `device`, `title`, `zoom`, `tools`, `picker`  | `page` (below), `click` a tool, `select` a picker's choice with its id                        |

A `code`'s text is typed, coloured and searched in the client, which takes
`draft`, else `text`, whenever `rev` changes, and tells the extension the
edit when it starts or stops differing from `text`, once typing settles, and
before any other event from its tree, so a tab switch never overtakes it. A
`code` whose tree shows several files gives each its own node id.
A `log` is cut into blocks by the client, from the shell's screen, pairing
the newest command with the newest prompt. A field with `live` reports
`change` as it is typed. A field with `lines` above 1 (`.lines(n)`, up to
twelve) is that many lines tall, for a key or a certificate; Return breaks
the line and Cmd+Return submits it. A one-line field with `blur`
(`.submits_on_blur()`) also submits what it holds when it loses focus, as a
password saves once typed and left; Return there just leaves it.

A few more shapes for a settings form:

- A button with `pick` (`.picks_file(title)`) opens the OS file dialog
  titled `title` instead of telling `click`; a file chosen comes back as
  `submit` on the button's id with the file's path. Nothing comes back when
  the dialog is cancelled, or where there is none (the web client).
- A choice with `dropdown` (`.dropdown()`) is a field showing the option
  chosen that opens a menu of the options, rather than a row of chips; it
  reports `select` the same way. The terminal and the platform's own
  controls draw it as chips.
- An entry with `expanded` (`.expanded(open)`) is a disclosure: the whole
  row is pressed, with a chevron, and tells `toggle` on the entry's id with
  `true` to open or `false` to close. Open, its children show under the row
  rather than beside its title.
- Text with the `label` style is a form field's small, uppercase caption.

A `web` part is a native web view the client places and seats, with the
address bar, back and forward, and an artifact's review controls over it.
It loads `url` whenever that changes, so an extension changes the address it
asks for, not what the page wanders to. Its `page` events carry:

| `type`   | fields                   | when                                                      |
| -------- | ------------------------ | --------------------------------------------------------- |
| `state`  | `url`, `title`, `design` | the page moved or titled itself; `design` names a design  |
| `submit` | `text`                   | Enter in the address bar, as typed; an error is shown     |
| `opened` | `url`                    | the page opened a window                                  |
| `go`     | `url`, `version`         | the client's own controls moved it, as to another version |

A `web` part needs the `web` permission.

A `video` part is a simulator's screen the client takes and draws itself, a
few times a second, inside its bezel, with the header's `tools`, a bar of
`controls` and the zoom under it; no frame passes through the extension.
`device` names the simulator shown; with `boot` set the client starts it
first and reports how that went, and `busy` spins the title meanwhile.
With `save` set it saves a screenshot of `device` to Downloads and reports
that too. Each tool and control has an `id`, `label` and `icon`, and may
add a shorter `tip`, `on`, `busy` (it spins) and `divider` (a line before
it). `zoom` is screen points per pixel, none fitting the screen to the pane;
`notice` is a line over the screen, and `picker` a list under the title
(`heading`, `loading`, `error`, `empty`, `choices` with `id`, `title`,
`detail`, a `status` tone and a `badge`); while it is `loading` the client
lists the simulators and reports them. Its `page` events carry:

| `type`    | fields               | when                                                   |
| --------- | -------------------- | ------------------------------------------------------ |
| `switch`  |                      | the title was clicked, to open or shut the picker      |
| `dismiss` |                      | the notice's ✕                                         |
| `zoom`    | `zoom`               | a zoom button; without `zoom`, fit to the pane         |
| `booted`  | `bootedHere`         | the client started `device`; whether it was off before |
| `failed`  | `message`            | it could not                                           |
| `listed`  | `devices` or `error` | the simulators a `loading` picker asked for            |
| `saved`   | `path` or `error`    | how the screenshot `save` asked for went               |

A `video` part needs the `video` permission; Vorn's own Device (`packages/core/extensions/device`) draws
the device pane with one. A Device row with `card` set is drawn over the
session card's phone button instead of in a pane: its picker, then the
button waiting while what was picked there starts.

A board holds columns, each holding its cards. A board of thousands cannot
travel as a tree: `board_of(Cards::Tasks, lanes)` names the core's task list
instead, each `lane(status, title)` a column, and the client reads the tasks
a page at a time, windows them, drags them between columns and opens them.
`graph()` places the workflow editor's canvas, which the client draws whole.

`examples/extensions/turns` draws a tree in every place, and
`examples/extensions/agent-footer` a card footer. Vorn's own Tasks
(`packages/core/extensions/tasks`) and Workflow editor
(`packages/core/extensions/workflow-editor`) draw their main views this way;
Files (`packages/core/extensions/files`) draws its pane and cards, Shell
blocks (`packages/core/extensions/blocks`) a shell's commands, the Browser
(`packages/core/extensions/browser`) a session's tabs over its page, and
the Device (`packages/core/extensions/device`) a session's simulator.

## The new API

### `host.rows`

`rows(collection) -> list<json>`: the rows the extension published to
`collection`, in the order they came. They outlive a restart of its service.

### `host.start-session`

Starts a session the way the app and the MCP tools do, and answers its id:

```wit
record session-start {
  project: string,          // the project's absolute folder
  agent: option<string>,    // an agent type; none starts a shell
  prompt: option<string>,   // what an agent is asked first
  worktree: bool,           // in a new worktree of the project
  branch: option<string>,   // that worktree's branch
  name: option<string>,     // the card's name
  headless: bool,           // an agent run with no terminal
}
```

A shell takes only `project` and `name`. A headless session names the
extension as what started it. `host.call("sessions.launch", ...)` takes the
same fields, with `project` defaulting to the call's project, and answers
`{sessionId}`.

### `storage.transaction`

`transaction(list<statement>) -> list<json>` runs the statements, each `{sql,
params}`, in order as one transaction on the extension's own tables: all of
them hold, or none does. Each answers its rows when it reads any, else how
many rows it changed. A failure names the statement, from 0: `statement 1:
...`. A statement may not open, commit or roll back a transaction itself.

### `blobs.*`: its own files

| function          | does                                                                           |
| ----------------- | ------------------------------------------------------------------------------ |
| `put(name, data)` | writes the file, whole or not at all                                           |
| `get(name)`       | the file, or none                                                              |
| `list(prefix)`    | `{name, size, modified}` of each file whose name starts with `prefix`, by name |
| `delete(name)`    | removes it; answers whether there was one                                      |

A name is segments of letters, digits, `.`, `_` and `-`, joined by `/`, none
starting with `.`, at most 200 bytes. A file holds up to 10 MiB, and an
extension keeps up to 256 MiB in all. Its files go when it is removed. Over
stdio, `data` and what `get` answers are base64 strings.

### Calls and events Files and Shell blocks use

- `files.list` (`{path}`) answers a folder's entries, `{name, path,
isDirectory}`, folders first, without what git ignores; `files.stat`
  (`{path}`) answers `{size, mtimeMs}` or null. Both under `files.read`.
- `shell.executables` answers the names of the programs on the shell's path,
  under `files.read`; `git.branches` (`{worktree}`) answers `{local,
current, isGitRepo}`, under `git.read`.
- `session.mark` carries `at` (when vornd read it) and `cwd`, the folder the
  shell last reported.

### Calls and events the workflow editor and Tasks use

- `files.read` with `encoding: "base64"` answers `{base64}`, a file's bytes,
  under `files.read`.
- `workflow.run` (`{workflowId, inputs}`) runs a workflow by hand,
  `workflow.resolveGate` (`{runId, nodeId, decision}`) answers a gate and
  `workflow.gates` lists the gates waiting for a person, under
  `workflows.write` and `workflows.read`. A workflow outside the caller's
  namespace (`<extension id>:`) also needs `process.run`, since its steps
  may run commands.
- The events `workflow.changed` (`{}`: a workflow was made, edited, switched
  or removed) and `workflow.gates` (`{runId, gates}`: a run's waiting gates
  changed).

### `web.call`: the Browser's web views

`web.call` (`{view, op, params}`) asks the app's web view `view`, a session
id, under the `web` permission.
vornd passes it to the app as `web:{op}` with `view` among the params, and
answers what the app does; with no app connected it fails.

| `op`      | params                           | answers                                      |
| --------- | -------------------------------- | -------------------------------------------- |
| `load`    | `url`                            | `{url}`, loaded again even when it was shown |
| `history` | `direction`: `back` or `forward` | `{url}`                                      |
| `read`    | `filter`, `cursor`, `limit`      | the page's nodes, a page at a time           |
| `text`    | `cursor`                         | the page's text, a page at a time            |
| `find`    | `text`, `limit`                  | the nodes that hold it                       |
| `act`     | what `browser:interact` takes    | what clicking, typing or scrolling did       |
| `console` | `limit`                          | its console messages                         |
| `network` | `limit`                          | its requests                                 |

The app refuses a `file:` address outside the session's project, on disk as
well as by name, and anything it would not load from the address bar.

### `video.call`: the Device's simulators

`video.call` (`{view, op, params}`) asks the app for the simulators on behalf
of the session `view`, under the `video` permission, which only the
extensions Vorn ships hold. vornd passes it to the app as `video:{op}` with
`view` among the params, and answers what the app does; with no app
connected it fails. `udid` names the simulator in all but `list`.

| `op`         | params             | answers                                                     |
| ------------ | ------------------ | ----------------------------------------------------------- |
| `list`       |                    | `[{udid, name, runtime, booted}]`, the available simulators |
| `boot`       | `udid`             | `{name, bootedHere}`, once it is up; waits for a slow boot  |
| `shutdown`   | `udid`             | `{ok}`                                                      |
| `launch`     | `udid`, `bundleId` | `{ok}`                                                      |
| `terminate`  | `udid`, `bundleId` | `{ok}`                                                      |
| `openUrl`    | `udid`, `url`      | `{ok}`                                                      |
| `install`    | `udid`, `path`     | `{ok}`                                                      |
| `screenshot` | `udid`             | `{data, width, height}`, a PNG in base64                    |
| `save`       | `udid`, `name`     | `{path}`, a screenshot saved to Downloads                   |

Any other `op`, such as reading the screen or the logs, fails with a message
saying the native app cannot yet.

### `remote.*`: the SSH built-in's hosts and keys

Vorn's SSH built-in edits the remote hosts and the stored SSH keys under the
`remote` permission. The hosts
stay in vornd's config (`remoteHosts`) and the keys in its key store, so
sessions on a remote host keep running with the built-in off.

| method             | params                                            | answers                                                     |
| ------------------ | ------------------------------------------------- | ----------------------------------------------------------- |
| `remote.hosts`     |                                                   | the hosts as the config keeps them                          |
| `remote.saveHosts` | `hosts`, the whole list                           | `null`, once the config is saved                            |
| `remote.test`      | `host`, optional `then` and `key`                 | `{success, message, durationMs}`; with `then`, see below    |
| `remote.keys`      |                                                   | the stored keys, without their private halves               |
| `remote.storeKey`  | `label`, `privateKey`, `publicKey`, `certificate` | the stored key                                              |
| `remote.importKey` | `path` (`~` is home)                              | the stored key, read from the file and its `.pub` beside it |
| `remote.deleteKey` | `id`                                              | `null`, the key and its secret gone                         |
| `remote.vault`     |                                                   | `{keychain}`: `false` when the OS keychain is missing       |

A test logs in for up to ten seconds, and a component answers one call at a
time, so with `then` `remote.test` answers `{testing: true}` at once and
later calls the extension's action `then` with `{key, result}`.

`remote.vault` says `keychain: false` only when vornd wanted the OS keychain
and found none, so its secrets sit in a private file instead; told not to
use it (`VORND_KEYCHAIN=0`), it says `true`.

Vorn's own Tasks keeps the board's tables, `tasks` and `task_source_links`,
which it is lent by name beside its own; no installed extension is.

## Limits

A component runs on a thread of its own in vornd, with:

- **Memory:** 64 MiB of linear memory. A grab past it ends the instance.
- **Time:** a call has the same deadline as a program's (60 s), its start 15 s.
  Past it the instance is stopped, and the call fails with `timeout`.
- **Nothing granted:** WASI preview 2 with no folders, no environment and no
  sockets. Only the API's imports reach out, held to the manifest.
- **Crashes:** a trap, a panic or a memory grab ends the instance and fails
  the call with `exited`, saying the last lines it wrote to stderr; the next
  call starts a fresh one. Five crashes in a minute and it waits until the
  minute passes, as a program does.

## Commands

| command                                                         | does                                                                                                     |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `vorn ext new <id> [--wasm] [--path <folder>] [--sdk <folder>]` | writes a package; `--sdk` builds against a local SDK in place of this release's                          |
| `vorn ext link [<folder>] [--watch]`                            | links a folder in place; a component is built first, and `--watch` rebuilds and reloads it on every save |
| `vorn ext compile <file.wasm> --into <folder>`                  | compiles a component into a cache; what Vorn runs at install, link and first start                       |
| `vorn ext install <git url>`                                    | shows what it runs and asks, then builds and installs it                                                 |
