import { describe, it, expect, vi, beforeEach } from 'vitest'

// Same seam the poll tests use: control the MCP client so the drain never
// spawns a server.
const callTool = vi.fn()
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: vi.fn(async () => ({ callTool }))
}))

import { backfillMcpConnection } from '../packages/server/src/connectors/mcp'
import type { ExternalItem, SourceConnection } from '../src/shared/types'

function makeConn(pollFilters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'Work items',
    filters: { command: 'npx', args: '[]', ...pollFilters },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-01-01T00:00:00Z'
  } as SourceConnection
}

function toolReturns(structuredContent: Record<string, unknown>) {
  callTool.mockResolvedValueOnce({ structuredContent, isError: false })
}

/** The shape a connector built with the SDK is wired as: the tool owns paging. */
const CURSORED = {
  pollTool: 'poll_workItem',
  itemsPath: 'items',
  idField: 'externalId',
  titleField: 'title',
  urlField: 'url',
  timestampField: 'updatedAt',
  cursorArg: 'cursor',
  cursorPath: 'nextCursor'
}

async function drain(conn: SourceConnection): Promise<ExternalItem[]> {
  const seen: ExternalItem[] = []
  await backfillMcpConnection(conn, (item) => seen.push(item))
  return seen
}

beforeEach(() => {
  callTool.mockReset()
})

describe('backfillMcpConnection', () => {
  it('says a connection with no poll tool is not a task source', async () => {
    // Reporting "imported 0" would read as "there was nothing there", which is
    // a different and wrong answer.
    await expect(backfillMcpConnection(makeConn({}), () => {})).rejects.toThrow(
      /no poll tool configured/
    )
    expect(callTool).not.toHaveBeenCalled()
  })

  it('follows the tool across pages, starting from no cursor', async () => {
    // The point of a backfill: import what the cron cursor already passed over.
    toolReturns({
      items: [{ externalId: '1', title: 'One', updatedAt: '2026-01-01T00:00:00Z' }],
      nextCursor: 'page-2'
    })
    toolReturns({
      items: [{ externalId: '2', title: 'Two', updatedAt: '2026-01-02T00:00:00Z' }],
      nextCursor: 'page-3'
    })
    toolReturns({ items: [], nextCursor: 'page-4' })

    const items = await drain(makeConn(CURSORED))

    expect(items.map((i) => i.externalId)).toEqual(['1', '2'])
    expect(callTool).toHaveBeenCalledTimes(3)
    // First call carries no cursor at all; the seed is the absence of one.
    expect(callTool.mock.calls[0][0].arguments).not.toHaveProperty('cursor')
    expect(callTool.mock.calls[1][0].arguments).toMatchObject({ cursor: 'page-2' })
  })

  it('starts from the beginning even when the connection seeds a cursor', async () => {
    // pollMcpConnection treats a cursor already in pollArgs as the seed for
    // the very first poll, which is right for ordinary polling and wrong here:
    // honouring it would start the import partway through and silently skip
    // everything before it, which is the whole thing a backfill is for.
    toolReturns({ items: [{ externalId: '1', updatedAt: '2026-01-01T00:00:00Z' }] })

    await drain(makeConn({ ...CURSORED, pollArgs: '{"cursor":"2026-06-01","limit":10}' }))

    const args = callTool.mock.calls[0][0].arguments
    expect(args).not.toHaveProperty('cursor')
    // Everything else the connection configured still goes through.
    expect(args).toMatchObject({ limit: 10 })
  })

  it("leaves the seed alone once it is following the tool's own cursor", async () => {
    toolReturns({
      items: [{ externalId: '1', updatedAt: '2026-01-01T00:00:00Z' }],
      nextCursor: 'p2'
    })
    toolReturns({ items: [] })

    await drain(makeConn({ ...CURSORED, pollArgs: '{"cursor":"2026-06-01"}' }))

    expect(callTool.mock.calls[1][0].arguments).toMatchObject({ cursor: 'p2' })
  })

  it('leaves unparseable poll args for the poll itself to report', async () => {
    // Swallowing it here would replace pollMcpConnection's specific message
    // with a confusing one about backfill.
    await expect(
      backfillMcpConnection(makeConn({ ...CURSORED, pollArgs: '{not json' }), () => {})
    ).rejects.toThrow(/pollArgs/i)
  })

  it('stops when the tool stops moving its cursor', async () => {
    // Otherwise a tool that answers the same page forever would drain until the
    // page cap, thousands of calls later.
    toolReturns({
      items: [{ externalId: '1', updatedAt: '2026-01-01T00:00:00Z' }],
      nextCursor: 'x'
    })
    toolReturns({
      items: [{ externalId: '2', updatedAt: '2026-01-02T00:00:00Z' }],
      nextCursor: 'x'
    })

    const items = await drain(makeConn(CURSORED))

    expect(items.map((i) => i.externalId)).toEqual(['1', '2'])
    expect(callTool).toHaveBeenCalledTimes(2)
  })

  it('reads one page from a connection that dedupes client-side', async () => {
    // Without a cursorArg the tool returns its whole window every call, so a
    // second pass would re-read the same items indefinitely. It still hands
    // back a cursor — the newest timestamp it saw — and following that as
    // though it were a paging token is exactly the trap.
    toolReturns({
      items: [
        { externalId: '1', title: 'One', updatedAt: '2026-01-01T00:00:00Z' },
        { externalId: '2', title: 'Two', updatedAt: '2026-01-02T00:00:00Z' }
      ]
    })

    const items = await drain(
      makeConn({
        pollTool: 'poll_thing',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt'
      })
    )

    expect(items).toHaveLength(2)
    expect(callTool).toHaveBeenCalledTimes(1)
  })

  it('carries the fields backfill upserts on', async () => {
    toolReturns({
      items: [
        {
          externalId: 'ADO-7',
          title: 'Disk full',
          url: 'https://dev.azure.com/x/_workitems/edit/7',
          description: 'It is full.',
          status: 'Active',
          updatedAt: '2026-07-01T00:00:00Z'
        }
      ]
    })

    const [item] = await drain(
      makeConn({
        pollTool: 'poll_workItem',
        itemsPath: 'items',
        idField: 'externalId',
        titleField: 'title',
        urlField: 'url',
        timestampField: 'updatedAt'
      })
    )

    expect(item).toMatchObject({
      externalId: 'ADO-7',
      title: 'Disk full',
      url: 'https://dev.azure.com/x/_workitems/edit/7',
      description: 'It is full.',
      // Mapped through the connection's statusMapping by the caller, so the
      // raw upstream value has to survive this far.
      status: 'Active',
      updatedAt: '2026-07-01T00:00:00Z'
    })
  })

  it('keeps the whole item, so nothing a workflow templates is lost', async () => {
    toolReturns({
      items: [{ externalId: '1', severity: 3, updatedAt: '2026-01-01T00:00:00Z' }]
    })

    const [item] = await drain(
      makeConn({ pollTool: 'p', itemsPath: 'items', idField: 'externalId' })
    )

    expect(item.metadata).toMatchObject({ severity: 3 })
  })

  it('reports a failing tool rather than importing nothing quietly', async () => {
    callTool.mockResolvedValueOnce({ isError: true, content: [{ type: 'text', text: 'boom' }] })
    await expect(backfillMcpConnection(makeConn(CURSORED), () => {})).rejects.toThrow(/boom/)
  })
})
