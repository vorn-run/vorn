# @vornrun/connector-sdk

Build a Vorn pull connector in TypeScript and share it as an ordinary npm
package. A connector built with this SDK runs as an MCP stdio server, and
Vorn's generic MCP connector already knows how to talk to one.

```bash
npm install @vornrun/connector-sdk
```

## Write a connector

```ts
// src/index.ts
import { defineConnector } from '@vornrun/connector-sdk'

export default defineConnector({
  id: 'acme',
  name: 'Acme Tickets',
  version: '1.0.0',
  config: [
    { key: 'apiToken', label: 'API token', required: true, secret: true },
    { key: 'baseUrl', label: 'Base URL', default: 'https://api.acme.test' }
  ],
  triggers: [
    {
      type: 'newTicket',
      label: 'New ticket',
      description: 'Tickets created or updated since the last poll',
      // Say how new items are recognized and the SDK owns the cursor for you.
      dedupe: 'timestamp',
      async fetch({ config, since, limit }) {
        const url = new URL('/tickets', config.baseUrl)
        if (since) url.searchParams.set('updated_after', since)
        url.searchParams.set('per_page', String(limit ?? 100))

        const response = await fetch(url, {
          headers: { authorization: `Bearer ${config.apiToken}` }
        })
        if (!response.ok) throw new Error(`Acme returned ${response.status}`)
        const tickets = (await response.json()) as AcmeTicket[]

        return tickets.map((ticket) => ({
          externalId: ticket.id,
          title: ticket.subject,
          url: ticket.html_url,
          description: ticket.body,
          status: ticket.state,
          updatedAt: ticket.updated_at,
          data: { priority: ticket.priority }
        }))
      }
    }
  ],
  actions: [
    {
      type: 'closeTicket',
      label: 'Close ticket',
      inputs: [{ key: 'id', label: 'Ticket id', required: true }],
      // Optional: declared outputs show up in Vorn's `{{steps.…}}` autocomplete.
      // Undeclared fields are still returned.
      outputs: [{ key: 'closed' }],
      async run({ id }, { config }) {
        await fetch(`${config.baseUrl}/tickets/${id}/close`, {
          method: 'POST',
          headers: { authorization: `Bearer ${config.apiToken}` }
        })
        return { closed: id }
      }
    }
  ]
})
```

Then a two-line bin:

```ts
// src/bin.ts
import { serveConnector } from '@vornrun/connector-sdk'
import connector from './index'

await serveConnector(connector)
```

Publish it like any other package (`"bin": { "acme-connector": "dist/bin.js" }`).

`vorn-connector new acme` writes all of the above — package, entry, definition,
a test that needs no network — already building, checking and packing.

## Declare an action instead of writing one

Most actions put arguments into a request and keep part of the answer. Say that
and the SDK does the rest: `{{args.x}}` and `{{config.y}}` are filled in, an
argument nobody supplied is left out, a failed status is raised with what the
body said, and `postReceive` reshapes what came back.

```ts
{
  type: 'createIssue',
  label: 'Create issue',
  idempotent: false,
  inputs: [{ key: 'title', label: 'Title', required: true }],
  request: {
    method: 'POST',
    url: '{{config.baseUrl}}/issues',
    headers: { authorization: 'Bearer {{config.apiToken}}' },
    body: { title: '{{args.title}}' }
  },
  // pick · rename · flatten · filter · map, applied left to right.
  postReceive: [{ op: 'pick', keys: ['id', 'html_url'] }]
}
```

Add `paginate` to follow every page rather than the first — `{ kind: 'cursor',
cursorPath: 'next', param: 'cursor' }`, `{ kind: 'page', param: 'page' }` or
`{ kind: 'link' }` — and the pages arrive concatenated.

Retry, backoff and rate-limit handling are applied for you, to declared
requests and to `context.fetch` alike. A read is always retried; a write only
when the action declares `idempotent: true`, because repeating a create makes a
second one. Prefer `context.fetch` over the global one in a hand-written action.

## Offer a field the choices it has

A `select` with fixed choices carries them; one whose choices only exist against
a live connection names a set the connector serves.

```ts
options: { channels: async ({ config, fetch }) => ['general', 'random'] },
actions: [{
  type: 'post',
  label: 'Post',
  inputs: [{ key: 'channel', label: 'Channel', type: 'select', loadOptions: 'channels' }],
  request: { method: 'POST', url: '{{config.baseUrl}}/post/{{args.channel}}' }
}]
```

A `json` argument arrives parsed. `builderHint` on a field is a note for whoever
writes the next connector, not for whoever runs this one.

## Poll a database instead of an API

Nothing about a trigger is HTTP-specific — it just returns items. A SQL pull
connector is the same shape:

```ts
import { defineConnector } from '@vornrun/connector-sdk'
import postgres from 'postgres'

export default defineConnector({
  id: 'orders-db',
  name: 'Orders database',
  config: [{ key: 'databaseUrl', label: 'Database URL', required: true, secret: true }],
  triggers: [
    {
      type: 'newOrder',
      label: 'New order',
      dedupe: 'timestamp',
      async fetch({ config, since, limit }) {
        const sql = postgres(config.databaseUrl!)
        try {
          const rows = await sql`
            SELECT id, reference, status, updated_at
            FROM orders
            WHERE updated_at >= ${since ?? '1970-01-01'}
            ORDER BY updated_at ASC
            LIMIT ${limit ?? 200}
          `
          return rows.map((row) => ({
            externalId: row.id,
            title: `Order ${row.reference}`,
            status: row.status,
            updatedAt: row.updated_at
          }))
        } finally {
          await sql.end()
        }
      }
    }
  ]
})
```

Two rules make a pull trigger reliable, and the SDK enforces both:

1. `externalId` must be stable — Vorn dedupes on it, so a changing id means
   duplicate work items.
2. `updatedAt` must be monotonic and ISO-comparable — the cursor advances from
   it. Use `>=` when filtering on `since` and sort ascending; returning a few
   items again is free, because the SDK drops anything already delivered.

## Dedupe strategies

`dedupe` tells the SDK how to recognize new items, and it then owns the cursor
— including the case that quietly breaks hand-written connectors, where several
items share the newest timestamp and are either dropped forever (`>`) or
redelivered on every poll (`>=`).

| strategy    | your `fetch` receives | use it when                                      |
| ----------- | --------------------- | ------------------------------------------------ |
| `timestamp` | `since`               | the source has a reliable "last changed" field   |
| `lastItem`  | `lastItemId`          | a newest-first feed with no dependable timestamp |

```ts
{
  type: 'newPost',
  label: 'New post',
  dedupe: 'lastItem',
  // Return the feed newest-first; the SDK stops at the last id it delivered.
  fetch: ({ config }) => fetchFeed(config)
}
```

A first poll never drains the whole history — it delivers one page and starts
tracking from there.

## Paging a backlog by hand

When a source's paging cannot be expressed as "everything since X", implement
`poll` instead of `fetch` and own the cursor yourself:

```ts
async poll({ cursor, config }) {
  const page = Number(cursor ?? '1')
  const { items, totalPages } = await fetchPage(config, page)
  return {
    items: items.map(toItem),
    nextCursor: String(page + 1),
    hasMore: page < totalPages
  }
}
```

A cursor that does not advance is rejected rather than looped on.

## Check it before shipping

`vorn-connector check` verifies a connector against the contract Vorn relies
on — most importantly that re-polling with its own cursor does not redeliver
items it already handed over:

```console
$ npx vorn-connector check ./dist/index.js
error  trigger newTicket: re-polling with its own nextCursor returned 3 already-delivered item(s), starting with "1042" [redelivers-items]
warn   action closeTicket: does not declare `idempotent`, so an agent cannot tell whether retrying is safe [missing-idempotent]

1 error(s), 1 warning(s)
```

It exits non-zero on errors, so it works as a CI gate. Add `sample` items to a
trigger and they are replayed through the real dedupe pipeline, so a connector
can be checked before anyone has credentials for it; pass `--live` to poll the
real source instead.

```ts
{
  type: 'newTicket',
  label: 'New ticket',
  dedupe: 'timestamp',
  sample: [{ externalId: '1', title: 'Example ticket', updatedAt: '2026-01-01T00:00:00.000Z' }],
  fetch: ({ config, since }) => fetchTickets(config, since)
}
```

## Test it without running the app

```ts
import { createConnectorHarness } from '@vornrun/connector-sdk'
import connector from '../src/index'

const harness = createConnectorHarness(connector, {
  config: { apiToken: 'test' },
  now: () => '2026-08-05T00:00:00.000Z'
})

test('emits normalized tickets', async () => {
  const page = await harness.poll('newTicket')
  expect(page.items[0]).toMatchObject({ externalId: '1', status: 'open' })
})

test('does not redeliver the same backlog forever', async () => {
  // Polls twice, carrying the watermark forward exactly as Vorn does.
  expect(await harness.pollTwice('newTicket')).toEqual([])
})
```

`harness.drain()` walks every page, `harness.execute()` runs an action, and
`harness.manifest()` returns what Vorn will see.

## Install it in Vorn

**Settings → Connectors → MCP → From a package**, then type the package name.

Vorn starts the connector once, asks it to describe itself, and fills in the
connection settings from the answer. All that is left on screen is the
connector's own name, its triggers, and the config it declared.

Nothing needs transcribing: `pollTool`, `itemsPath`, `idField`,
`timestampField`, `titleField`, `urlField`, `cursorArg` and `cursorPath` all
come from the manifest. `cursorArg` is what makes the dedupe strategy
load-bearing — Vorn hands the connector back its own cursor on every poll and
fires for whatever it returns, rather than re-filtering by timestamp itself.

Because the connector is a normal npm package, versions are pinned by the
package spec and upgrades are a version bump — no separate registry.

To see the same values on the command line, or to wire a connection up by
hand:

```bash
npx vorn-connector setup ./dist/index.js
```

## Pack it as a file

`vorn-connector pack` builds a single installable file: the manifest plus one
bundled entry with every dependency inlined.

```bash
npx vorn-connector pack ./dist/index.js --out ./release
# → release/acme-1.2.3.vorn.tgz
```

Packing runs `check` first, then two gates a pack must pass: the source package
declares no install-time scripts, and nothing was left outside the bundle. A
pack installs by copying files, so it works with no registry reachable — drop
it on **Settings → Connectors** and Vorn launches it from disk.

### Ship an icon

Without one, a connector shows the generic MCP glyph and is hard to pick out
of a list of connections.

```ts
defineConnector({
  id: 'acme',
  name: 'Acme',
  icon: {
    viewBox: '0 0 24 24',
    paths: ['M12 2 2 22h20L12 2z']
  }
  // ...
})
```

Path data only — no markup, no `<svg>` wrapper, no external references. Vorn
draws these as `<path d="...">` inside an element it owns, so an icon can
never contribute markup to the app rendering it. Anything that is not path
data is rejected by `defineConnector` at import time, and again when Vorn
reads the manifest.

Paths are filled with `currentColor`, so the icon picks up the surrounding
text color instead of fighting the theme.

## CLI

```
vorn-connector new <id>                   Scaffold a new connector, ready to build
vorn-connector manifest <module>          Print the manifest as JSON
vorn-connector setup <module> [trigger]   Print the Vorn connection settings
vorn-connector poll <module> <trigger>    Run one poll against the environment
vorn-connector check <module>             Verify the connector against the contract
vorn-connector pack <module>              Build an installable .vorn.tgz pack
vorn-connector serve <module>             Serve on stdio (what Vorn runs)
```

`new` accepts `--out <dir>` and `--name "Display Name"`; `pack` accepts
`--out <dir>`.

`poll` accepts `--since <iso>` and `--limit <n>`, and reads the connector's
declared config from your shell environment — the fastest way to confirm
credentials and field mapping before wiring anything up.

`check` runs against declared `sample` data by default and takes `--live` to
poll the real source instead.
