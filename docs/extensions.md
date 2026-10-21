# Writing an extension

An extension is a folder with a `vorn-extension.toml` and whatever runs it:
a program speaking JSON-RPC over stdio (any language), a Wasm component
(Rust), or one-shot commands. All three are held to one API, `vorn:extension`,
defined in `packages/core/crates/extensions/wit/extension.wit`. The
extensions Vorn ships use the same API, under the same permissions, as
yours: there is no private method or hook. [Wasm extensions](wasm-extensions.md)
covers components, the element tree and the methods in detail.

## The API version

The API is `vorn:extension@1.0`. The WIT package carries it
(`package vorn:extension@1.0.0`), and so does the stdio protocol: its
mapping, `wit/stdio.json`, names the same package, and a program is told
the version Vorn speaks when it starts.

- A **minor** version only adds: methods, events, optional fields. An
  extension built for `1.0` runs on a Vorn speaking `1.0` or any later `1.x`.
- A **major** version removes or changes something. An extension built for
  another major is refused until it is rebuilt.
- An extension built for a **newer** minor than this Vorn speaks is refused,
  naming both: `built for vorn:extension@1.1; this Vorn speaks
vorn:extension@1.0, so update Vorn to run it`.

## The manifest

```toml
id = "acme.hello"
name = "Hello"
version = "0.1.0"
api = "vorn:extension@1.0"   # the API it was built for
vorn = ">=1.0, <2"           # optional: the API versions it runs on
```

`api` is required. `vorn` is optional, a range of API versions: one or more
comparators separated by commas, each `>=`, `>`, `<=`, `<`, `=`, `^` or `~`
followed by a `MAJOR.MINOR` (or `MAJOR` or `MAJOR.MINOR.PATCH`); a bare
version means `^`. Both are checked when the extension is installed or
linked and each time Vorn loads it. A mistake names the field and its line:

```text
vorn-extension.toml:5: vorn: `=>1.0` starts with `=>`, which is not a comparator; use >=, >, <=, <, =, ^ or ~
vorn-extension.toml:4: api: built for vorn:extension@1.1; this Vorn speaks vorn:extension@1.0, so update Vorn to run it
```

## The handshake

- **A component** is checked before it is linked: the version of the
  `vorn:extension` interfaces it imports is its API, and one Vorn does not
  speak is refused with the message above.
- **A program** is started with `guest.init`, whose `ctx.api` is the version
  Vorn speaks. A program built for a newer one answers with an error; Vorn
  shows its message and does not start it.
- **A one-shot command** is checked by its manifest alone.

## TypeScript

A program reads JSON-RPC from stdin, one message per line, and answers on
stdout. `vorn ext new acme.hello` writes one; its start, in TypeScript:

```ts
import { createInterface } from 'node:readline'

const API = 'vorn:extension@1.0'
const parts = (v: string) => (v.split('@')[1] ?? '').split('.').map(Number)
const runsOn = (api: string) => {
  const [host, built] = [parts(api), parts(API)]
  return host[0] === built[0] && host[1] >= built[1]
}
const send = (m: object) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...m }) + '\n')

createInterface({ input: process.stdin }).on('line', (line) => {
  const m = JSON.parse(line)
  if (m.method === 'guest.init') {
    const api: string = m.params.ctx.api
    if (runsOn(api)) return send({ id: m.id, result: null })
    const message = `built for ${API}; this Vorn speaks ${api}, so update Vorn to run it`
    return send({ id: m.id, error: { code: -32000, message, data: { code: 'version' } } })
  }
  if (m.method === 'guest.handle') {
    // m.params.method is the action, m.params.args its arguments.
    return send({ id: m.id, result: { ok: true } })
  }
})
```

A program calls Vorn with `host.call` (`{method, args}`), under the
permissions its manifest declares. A pull connector can use
`@vornrun/connector-sdk` (`packages/connector-sdk`) instead.

## Rust

A component builds on `vorn-extension-sdk` alone, which is built for the
API it ships with: rebuilding against a newer SDK moves the extension to the
newer API. `Init::api` is the version Vorn speaks.

```rust
use vorn_extension_sdk::{export, Ctx, Error, Extension, Host, Init, Value};

struct Hello;

impl Extension for Hello {
    fn init(init: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        // Vorn has already refused a component built for an API it does not speak.
        let _speaks = &init.api;
        Ok(Hello)
    }
    fn handle(&mut self, host: &mut dyn Host, method: &str, _: Value, _: &Ctx) -> Result<Value, Error> {
        match method {
            "sessions" => host.call("sessions.list", Value::Null),
            other => Err(Error::unknown(other)),
        }
    }
}

export!(Hello);
```

`vorn ext new acme.hello --wasm` writes the whole crate, with tests against
a fake Vorn.

## Shell

An action can be a command run once per call: it reads one request on stdin
and writes one answer, both JSON-RPC with id 1.

```toml
id = "acme.greet"
name = "Greet"
version = "0.1.0"
api = "vorn:extension@1.0"

[actions.greet]
title = "Say hello"
command = ["bash", "greet.sh"]
```

```sh
#!/usr/bin/env bash
read -r request
name=$(printf '%s' "$request" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')
printf '{"jsonrpc":"2.0","id":1,"result":{"greeting":"hello %s"}}\n' "${name:-there}"
```

## What you may call

Every bridge method is public: the list is `CORE_METHODS` in
`packages/core/crates/extensions/src/gate.rs`, each with the permission it
needs, and the events are `CORE_EVENTS` in `events.rs`. A call the manifest
does not declare the permission for is refused, naming both; the person
installing sees every permission first.

An extension's workflows live in its own namespace: the ids `workflow.seed`
inserts and the prefix `workflow.unseed` removes start with its id and a
colon (`acme.tickets:`), and `workflow.run` runs only those. Running anyone
else's workflow, or seeding a step that runs commands or agents (a script, an
agent, or a type no extension declares, which runs as an agent), also needs
`process.run`, which the install sheet shows as running programs. Vorn's own
workflow ids start with `import:`, `connector:` and `system:`, so `import`,
`connector` and `system` cannot be an extension's id, and ids starting with
`vorn.` belong to the extensions Vorn ships.

The core is frozen against that API: `packages/core/freeze.toml` lists every
crate of the core by its area and every feature still left in it, and
`cargo test -p vorn-extensions --test freeze` fails when an extension Vorn
ships reaches past the public API or a new feature lands in the core.
