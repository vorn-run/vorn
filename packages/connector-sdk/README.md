# @vornrun/connector-sdk

Build a Vorn pull connector in TypeScript and share it as an ordinary npm
package. No marketplace, no plugin host, no changes to Vorn itself: a connector
built with this SDK runs as an MCP stdio server, and Vorn's generic MCP
connector already knows how to talk to one.

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
      async poll({ config, since, limit }) {
        const url = new URL('/tickets', config.baseUrl)
        if (since) url.searchParams.set('updated_after', since)
        url.searchParams.set('per_page', String(limit ?? 100))

        const response = await fetch(url, {
          headers: { authorization: `Bearer ${config.apiToken}` }
        })
        if (!response.ok) throw new Error(`Acme returned ${response.status}`)
        const tickets = (await response.json()) as AcmeTicket[]

        return {
          items: tickets.map((ticket) => ({
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
      async poll({ config, since, limit }) {
        const sql = postgres(config.databaseUrl!)
        try {
          const rows = await sql`
            SELECT id, reference, status, updated_at
            FROM orders
            WHERE updated_at > ${since ?? '1970-01-01'}
            ORDER BY updated_at ASC
            LIMIT ${limit ?? 200}
          `
          return {
            items: rows.map((row) => ({
              externalId: row.id,
              title: `Order ${row.reference}`,
              status: row.status,
              updatedAt: row.updated_at
            }))
          }
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
2. `updatedAt` must be monotonic and ISO-comparable — Vorn advances its poll
   cursor from it. Sort ascending by that column and honor `since`.

## Paging a backlog

Return `hasMore: true` with a `nextCursor`, and Vorn (or `drainPoll` in tests)
will keep pulling bounded pages until the backlog is drained:

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

| Field      | Value                                                                                                                                    |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Command    | `npx`                                                                                                                                    |
| Arguments  | `["-y", "@your-scope/acme-connector"]`                                                                                                   |
| Secret env | `{"API_TOKEN": "…"}`                                                                                                                     |
| Filters    | `pollTool: poll_newTicket`, `itemsPath: items`, `idField: externalId`, `timestampField: updatedAt`, `titleField: title`, `urlField: url` |

Because the connector is a normal npm package, versions are pinned by the
`npx` argument and upgrades are a version bump — no separate registry.

## CLI

```
vorn-connector manifest <module>          Print the manifest as JSON
vorn-connector setup <module> [trigger]   Print the Vorn connection settings
vorn-connector poll <module> <trigger>    Run one poll against the environment
vorn-connector serve <module>             Serve on stdio (what Vorn runs)
```

`poll` accepts `--since <iso>` and `--limit <n>`, and reads the connector's
declared config from your shell environment — the fastest way to confirm
credentials and field mapping before wiring anything up.
