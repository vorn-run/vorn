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
api = "vorn:extension@0.9"

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
selection, and boards. A program answers the same tree as JSON. The host
draws it on every client: the desktop, the web client, a terminal's cells
(`surface.cells`) and the phone.

Any contribution that draws says `tree = true`: a sidebar section, a card's
badges and footer, a sidebar row's marks (`decorations`), a pane, the
composer's strip, a settings page, a window and a dialog. Each is asked for
its tree with `ui.render` (`{point, surface}` in, `{tokens, tree}` out),
`point` being the contribution's id, so a tree contribution's id is unique
in its manifest. The host asks again whenever the extension's rows change,
and when the place shows.

A badge, a card footer and a sidebar row's marks draw once per session: the
tree's root is a box holding one child per session, its id the session's
id, and each card draws its own. `per_session` builds it; a session with no
child draws nothing. A sidebar section whose tree is an empty box does not
show.

A click, a toggle, a choice or a field's Return comes back as `ui.event`
(`{point, event}` in). Its answer, every field optional, says what follows:

| field    | does                                                                        |
| -------- | --------------------------------------------------------------------------- |
| `toast`  | a line that says what happened                                              |
| `error`  | a line that says what failed                                                |
| `patch`  | nodes changed in place, without a render                                    |
| `open`   | opens one of its dialogs, by id                                             |
| `window` | opens one of its windows by id, in place of the one open                    |
| `close`  | `true` closes the dialog or window the tree is in                           |
| `focus`  | a session to show in the main window, which comes forward                   |
| `show`   | `true` brings the main window forward                                       |
| `url`    | an `https:` page to open in the browser                                     |
| `signin` | opens Vorn's sign-in window: the target a `signin` method answers, and `id` |

`examples/extensions/turns` draws a tree in every place, and
`examples/extensions/agent-footer` a card footer.

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
