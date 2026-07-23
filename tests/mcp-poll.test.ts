import { describe, it, expect, vi, beforeEach } from 'vitest'

// Control the MCP client so pollMcpConnection → invokeMcpTool → getOrStartClient
// returns our canned tool result instead of spawning a real server.
const callTool = vi.fn()
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: vi.fn(async () => ({ callTool }))
}))

import { pollMcpConnection, MCP_POLL_EVENT } from '../packages/server/src/connectors/mcp'
import type { SourceConnection } from '../src/shared/types'

function makeConn(pollFilters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'ADO',
    filters: { command: 'npx', args: '[]', ...pollFilters },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-01-01T00:00:00Z'
  }
}

/** Canned tool result in the shape invokeMcpTool surfaces (structuredContent). */
function toolReturns(structuredContent: Record<string, unknown>) {
  callTool.mockResolvedValueOnce({ structuredContent, isError: false })
}

beforeEach(() => {
  callTool.mockReset()
})

describe('pollMcpConnection', () => {
  it('returns no events (and calls nothing) when no pollTool is configured', async () => {
    const result = await pollMcpConnection(makeConn({}))
    expect(result.events).toEqual([])
    expect(callTool).not.toHaveBeenCalled()
  })

  it('maps items at itemsPath into events with id/title/url/timestamp', async () => {
    toolReturns({
      pullRequests: [
        { pullRequestId: 7, title: 'Fix bug', url: 'https://x/7', creationDate: '2026-07-01' },
        { pullRequestId: 8, title: 'Add feature', url: 'https://x/8', creationDate: '2026-07-02' }
      ]
    })
    const result = await pollMcpConnection(
      makeConn({
        pollTool: 'list_pull_requests',
        itemsPath: 'pullRequests',
        idField: 'pullRequestId',
        titleField: 'title',
        urlField: 'url',
        timestampField: 'creationDate'
      })
    )
    expect(callTool).toHaveBeenCalledWith({ name: 'list_pull_requests', arguments: {} })
    expect(result.events).toHaveLength(2)
    expect(result.events[0]).toMatchObject({
      id: '7',
      type: MCP_POLL_EVENT,
      timestamp: '2026-07-01',
      data: { externalId: '7', title: 'Fix bug', url: 'https://x/7', pullRequestId: 7 }
    })
    // Cursor advances to the newest timestamp seen.
    expect(result.nextCursor).toBe('2026-07-02')
  })

  it('filters out items at or before the cursor when a timestampField is set', async () => {
    toolReturns({
      items: [
        { id: 'a', ts: '2026-07-01' },
        { id: 'b', ts: '2026-07-03' }
      ]
    })
    const result = await pollMcpConnection(
      makeConn({ pollTool: 'list', itemsPath: 'items', idField: 'id', timestampField: 'ts' }),
      '2026-07-02'
    )
    expect(result.events.map((e) => e.id)).toEqual(['b'])
    expect(result.nextCursor).toBe('2026-07-03')
  })

  it('skips items missing the timestampField when ordering is configured', async () => {
    toolReturns({
      items: [
        { id: 'a', ts: '2026-07-03' },
        { id: 'b' }, // no ts — can't be ordered, must not fire every poll
        { id: 'c', ts: '2026-07-01' }
      ]
    })
    const result = await pollMcpConnection(
      makeConn({ pollTool: 'list', itemsPath: 'items', idField: 'id', timestampField: 'ts' }),
      '2026-07-02'
    )
    // Only 'a' (ts > cursor) fires; 'b' (no ts) and 'c' (<= cursor) are skipped.
    expect(result.events.map((e) => e.id)).toEqual(['a'])
    expect(result.nextCursor).toBe('2026-07-03')
  })

  it('emits every item and sets no cursor when no timestampField is configured', async () => {
    toolReturns({ items: [{ id: 'a' }, { id: 'b' }] })
    const result = await pollMcpConnection(
      makeConn({ pollTool: 'list', itemsPath: 'items', idField: 'id' }),
      'ignored-cursor'
    )
    expect(result.events.map((e) => e.id)).toEqual(['a', 'b'])
    expect(result.nextCursor).toBeUndefined()
  })

  it('walks a nested itemsPath', async () => {
    toolReturns({ data: { value: [{ id: '1' }] } })
    const result = await pollMcpConnection(
      makeConn({ pollTool: 'list', itemsPath: 'data.value', idField: 'id' })
    )
    expect(result.events.map((e) => e.id)).toEqual(['1'])
  })

  it('falls back to a stable id when idField is missing', async () => {
    toolReturns({ items: [{ foo: 'bar' }] })
    const result = await pollMcpConnection(makeConn({ pollTool: 'list', itemsPath: 'items' }))
    expect(result.events[0].id).toBe(JSON.stringify({ foo: 'bar' }))
    expect(result.events[0].data.externalId).toBe(JSON.stringify({ foo: 'bar' }))
  })

  it('passes static pollArgs through to the tool', async () => {
    toolReturns({ items: [] })
    await pollMcpConnection(
      makeConn({ pollTool: 'list', itemsPath: 'items', pollArgs: '{"status":"active"}' })
    )
    expect(callTool).toHaveBeenCalledWith({
      name: 'list',
      arguments: { status: 'active' }
    })
  })

  it('throws when itemsPath does not resolve to an array', async () => {
    toolReturns({ notAnArray: true })
    await expect(
      pollMcpConnection(makeConn({ pollTool: 'list', itemsPath: 'missing' }))
    ).rejects.toThrow(/did not resolve to an array/)
  })

  it('throws when the tool reports an error', async () => {
    callTool.mockResolvedValueOnce({
      isError: true,
      content: [{ type: 'text', text: 'boom' }]
    })
    await expect(
      pollMcpConnection(makeConn({ pollTool: 'list', itemsPath: 'items' }))
    ).rejects.toThrow(/boom/)
  })

  it('throws on invalid pollArgs JSON', async () => {
    await expect(
      pollMcpConnection(makeConn({ pollTool: 'list', pollArgs: '{ not json' }))
    ).rejects.toThrow(/Invalid pollArgs JSON/)
    expect(callTool).not.toHaveBeenCalled()
  })

  it('throws when pollArgs is valid JSON but not an object', async () => {
    await expect(
      pollMcpConnection(makeConn({ pollTool: 'list', pollArgs: '[1, 2, 3]' }))
    ).rejects.toThrow(/must be a JSON object/)
    expect(callTool).not.toHaveBeenCalled()
  })
})
