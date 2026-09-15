import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Database from 'libsql'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { queryDb } from './helpers/database'

let dataDir: string
let dbFile: string

const query = <T>(fn: (d: Database.Database) => T): T => queryDb(dbFile, fn)

const POLL_FILTERS = {
  itemsPath: 'items',
  idField: 'externalId',
  timestampField: 'updatedAt',
  titleField: 'title',
  urlField: 'url',
  cursorArg: 'cursor',
  cursorPath: 'nextCursor'
}
const TOOLS = [{ name: 'vorn_connector_manifest' }, { name: 'echo' }]
const launch = (pkg: string) => ({ command: 'npx', args: `["-y","${pkg}"]`, env: '{}' })
const icon = '{"viewBox":"0 0 24 24","paths":["M0 0h24v24H0z"]}'

// The five connections as the database held them before the switch, plus two that must not move.
const ROWS: Array<{ id: string; name: string; filters: Record<string, unknown>; signedIn?: true }> =
  [
    {
      id: 'github-1',
      name: 'GitHub: New issue',
      filters: {
        ...launch('@vornrun/connector-github'),
        sdkConnectorId: 'github',
        sdkVersion: '0.2.0',
        sdkIcon: icon,
        pollTool: 'poll_issueCreated',
        ...POLL_FILTERS,
        discoveredTools: TOOLS
      }
    },
    {
      id: 'midjourney-1',
      name: 'Midjourney',
      filters: {
        command: 'node',
        args: '["/packs/midjourney/index.js"]',
        env: '{}',
        sdkConnectorId: 'midjourney',
        sdkVersion: '0.1.0',
        sdkIcon: icon,
        discoveredTools: TOOLS
      },
      signedIn: true
    },
    {
      id: 'substack-1',
      name: 'Substack: New post',
      filters: {
        ...launch('@vornrun/connector-substack'),
        secretEnv: 'Y2lwaGVydGV4dA==',
        sdkConnectorId: 'substack',
        sdkVersion: '0.2.0',
        sdkIcon: icon,
        pollTool: 'poll_newPost',
        ...POLL_FILTERS,
        discoveredTools: TOOLS
      },
      signedIn: true
    },
    {
      id: 'rss-implicit',
      name: 'RSS',
      filters: {
        sdkConnectorId: 'rss',
        sdkVersion: '0.1.1',
        sdkIcon: icon,
        implicit: true,
        discoveredTools: TOOLS
      }
    },
    {
      id: 'rss-1',
      name: 'RSS: New item',
      filters: {
        ...launch('@vornrun/connector-rss'),
        sdkConnectorId: 'rss',
        sdkVersion: '0.1.1',
        sdkIcon: icon,
        pollTool: 'poll_newItem',
        ...POLL_FILTERS,
        discoveredTools: TOOLS
      }
    },
    {
      id: 'catalog-server',
      name: 'Filesystem',
      filters: {
        ...launch('@modelcontextprotocol/server-filesystem'),
        sdkConnectorId: 'filesystem'
      }
    },
    {
      id: 'raw-mcp',
      name: 'Tickets',
      filters: {
        command: 'uvx',
        args: '["tickets-mcp"]',
        pollTool: 'list_things',
        itemsPath: 'rows'
      }
    }
  ]

function seed(): void {
  query((d) => {
    // Inbox and cursor rows need no workflow behind them to show where they moved.
    d.exec('PRAGMA foreign_keys = OFF')
    const insert = d.prepare(
      `INSERT INTO source_connections
         (id, connector_id, name, filters, sync_interval_minutes, status_mapping, created_at, signed_in_as, signed_in_at)
       VALUES (?, 'mcp', ?, ?, 5, '{}', '2026-09-01T00:00:00Z', ?, ?)`
    )
    for (const row of ROWS) {
      const at = row.signedIn ? '2026-09-12T00:00:00Z' : null
      insert.run(row.id, row.name, JSON.stringify(row.filters), row.signedIn ? 'Javier' : null, at)
    }
    const inbox = d.prepare(
      `INSERT INTO connector_inbox
         (workflow_id, connection_id, connector_id, event_id, event_type, event_timestamp, payload, available_at, created_at)
       VALUES (?, ?, 'mcp', 'e1', 'mcpPoll', '2026-09-12T00:00:00Z', '{}', '2026-09-12T00:00:00Z', '2026-09-12T00:00:00Z')`
    )
    inbox.run('wf-substack', 'substack-1')
    inbox.run('wf-raw', 'raw-mcp')
    d.prepare(
      `INSERT INTO connector_poll_state (workflow_id, connection_id, cursor) VALUES ('wf-substack', 'substack-1', 'c-42')`
    ).run()
    d.prepare(
      `INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '20')`
    ).run()
  })
}

interface Stored {
  connectorId: string
  filters: Record<string, unknown>
  signedInAs: string | null
}

const stored = (id: string): Stored =>
  query((d) => {
    const { connectorId, filters, signedInAs } = d
      .prepare(
        'SELECT connector_id AS connectorId, filters, signed_in_as AS signedInAs FROM source_connections WHERE id = ?'
      )
      .get(id) as { connectorId: string; filters: string; signedInAs: string | null }
    return { connectorId, filters: JSON.parse(filters) as Record<string, unknown>, signedInAs }
  })

const inboxConnector = (connectionId: string): string =>
  query(
    (d) =>
      (
        d
          .prepare('SELECT connector_id AS id FROM connector_inbox WHERE connection_id = ?')
          .get(connectionId) as { id: string }
      ).id
  )

const migrate = (): void => {
  initDatabase(dataDir)
  closeDatabase()
}

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-migration-21-'))
  dbFile = path.join(dataDir, 'vorn.db')
  migrate()
  seed()
  migrate()
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

const MCP_ONLY = ['discoveredTools', 'pollTool', ...Object.keys(POLL_FILTERS)]

describe('migration 21 — package connections belong to sdk', () => {
  it('moves every package connection to sdk under the id it already had', () => {
    for (const id of ['github-1', 'midjourney-1', 'substack-1', 'rss-implicit', 'rss-1']) {
      expect(stored(id).connectorId).toBe('sdk')
    }
  })

  it('names the trigger a connection polled, from the tool it used to call', () => {
    expect(stored('github-1').filters.sdkTrigger).toBe('issueCreated')
    expect(stored('substack-1').filters.sdkTrigger).toBe('newPost')
    expect(stored('rss-1').filters.sdkTrigger).toBe('newItem')
    expect(stored('midjourney-1').filters).not.toHaveProperty('sdkTrigger')
    expect(stored('rss-implicit').filters).not.toHaveProperty('sdkTrigger')
  })

  it('drops what only MCP needed and keeps everything else', () => {
    const substack = stored('substack-1').filters
    for (const key of MCP_ONLY) expect(substack).not.toHaveProperty(key)
    expect(substack).toEqual({
      ...launch('@vornrun/connector-substack'),
      secretEnv: 'Y2lwaGVydGV4dA==',
      sdkConnectorId: 'substack',
      sdkVersion: '0.2.0',
      sdkIcon: icon,
      sdkTrigger: 'newPost'
    })
    expect(stored('rss-implicit').filters).toEqual({
      sdkConnectorId: 'rss',
      sdkVersion: '0.1.1',
      sdkIcon: icon,
      implicit: true
    })
  })

  it('keeps who a connection is signed in as', () => {
    expect(stored('midjourney-1').signedInAs).toBe('Javier')
    expect(stored('substack-1').signedInAs).toBe('Javier')
  })

  it('leaves a catalog MCP server and a raw MCP server exactly as they were', () => {
    for (const id of ['catalog-server', 'raw-mcp']) {
      const row = ROWS.find((entry) => entry.id === id)!
      expect(stored(id)).toEqual({ connectorId: 'mcp', filters: row.filters, signedInAs: null })
    }
  })

  it('moves the inbox rows of a package connection and no others, leaving cursors alone', () => {
    expect(inboxConnector('substack-1')).toBe('sdk')
    expect(inboxConnector('raw-mcp')).toBe('mcp')
    const cursor = query(
      (d) =>
        (
          d
            .prepare("SELECT cursor FROM connector_poll_state WHERE workflow_id = 'wf-substack'")
            .get() as { cursor: string }
        ).cursor
    )
    expect(cursor).toBe('c-42')
  })

  it('records the new version and changes nothing when it runs again', () => {
    const version = () =>
      query(
        (d) =>
          (
            d.prepare("SELECT value FROM schema_meta WHERE key = 'schema_version'").get() as {
              value: string
            }
          ).value
      )
    expect(Number(version())).toBeGreaterThanOrEqual(21)
    const before = ROWS.map((row) => stored(row.id))
    migrate()
    expect(ROWS.map((row) => stored(row.id))).toEqual(before)
  })
})
