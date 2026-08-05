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

Run the CLI to get the exact connection settings:

```bash
npx vorn-connector setup ./dist/index.js
```

It prints the values for **Settings → Connectors → MCP → New connection**:

| Field      | Value                                                                                                                                                                                   |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Command    | `npx`                                                                                                                                                                                   |
| Arguments  | `["-y", "@your-scope/acme-connector"]`                                                                                                                                                  |
| Secret env | `{"API_TOKEN": "…"}`                                                                                                                                                                    |
| Filters    | `pollTool: poll_newTicket`, `itemsPath: items`, `idField: externalId`, `timestampField: updatedAt`, `titleField: title`, `urlField: url`, `cursorArg: cursor`, `cursorPath: nextCursor` |

`cursorArg` is what makes the dedupe strategy load-bearing: Vorn hands the
connector back its own cursor on every poll and fires for whatever it returns,
rather than re-filtering the results by timestamp itself.

Because the connector is a normal npm package, versions are pinned by the
`npx` argument and upgrades are a version bump — no separate registry.

## CLI

```
vorn-connector manifest <module>          Print the manifest as JSON
vorn-connector setup <module> [trigger]   Print the Vorn connection settings
vorn-connector poll <module> <trigger>    Run one poll against the environment
vorn-connector check <module>             Verify the connector against the contract
vorn-connector serve <module>             Serve on stdio (what Vorn runs)
```

`poll` accepts `--since <iso>` and `--limit <n>`, and reads the connector's
declared config from your shell environment — the fastest way to confirm
credentials and field mapping before wiring anything up.

`check` runs against declared `sample` data by default and takes `--live` to
poll the real source instead.
