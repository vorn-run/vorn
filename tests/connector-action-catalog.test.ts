import { describe, it, expect, vi } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'
import { SDK_FILTER_KEYS } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const { mcpConnectionActions, mcpToolToConnectorAction } =
  await import('../packages/server/src/connectors/mcp')
const { isReservedSdkTool, MANIFEST_TOOL, PREFLIGHT_TOOL, pollToolName } =
  await import('../packages/server/src/connectors/sdk-tools')

/** The tools a packed SDK connector with one trigger and one action really serves. */
const PACK_TOOLS = [
  { name: MANIFEST_TOOL, description: 'Describe this connector' },
  { name: PREFLIGHT_TOOL, description: 'Report readiness' },
  { name: pollToolName('tick'), description: 'Poll Pack Demo for Tick' },
  { name: 'echo', title: 'Echo', description: 'Return the given message.' }
]

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'Pack Demo',
    filters,
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z'
  } as SourceConnection
}

describe('reserved SDK tool names', () => {
  it('hides only the poll tools a connector declared, when its triggers are known', () => {
    expect(isReservedSdkTool(pollToolName('tick'), ['tick'])).toBe(true)
    // A connector may genuinely offer an action that starts this way.
    expect(isReservedSdkTool('poll_status', ['tick'])).toBe(false)
    expect(isReservedSdkTool('poll_status')).toBe(true)
    expect(isReservedSdkTool(MANIFEST_TOOL, ['tick'])).toBe(true)
  })

  it('knows the three kinds of plumbing a connector serves', () => {
    expect(isReservedSdkTool(MANIFEST_TOOL)).toBe(true)
    expect(isReservedSdkTool(PREFLIGHT_TOOL)).toBe(true)
    expect(isReservedSdkTool(pollToolName('tick'))).toBe(true)
    expect(isReservedSdkTool(pollToolName('issueCreated'))).toBe(true)
  })

  it('leaves an ordinary action alone', () => {
    expect(isReservedSdkTool('echo')).toBe(false)
    expect(isReservedSdkTool('createIssue')).toBe(false)
  })
})

describe('actions offered for a connection', () => {
  it('hides the plumbing a packaged connector serves', () => {
    const actions = mcpConnectionActions(
      connection({ discoveredTools: PACK_TOOLS, [SDK_FILTER_KEYS.connectorId]: 'packdemo' })
    )
    expect(actions.map((a) => a.type)).toEqual(['echo'])
  })

  it('names the action by its label, keeping the tool name as the type', () => {
    const [echo] = mcpConnectionActions(
      connection({ discoveredTools: PACK_TOOLS, [SDK_FILTER_KEYS.connectorId]: 'packdemo' })
    )
    expect(echo.label).toBe('Echo')
    expect(echo.type).toBe('echo')
  })

  it('keeps every tool for a raw MCP server, where nothing is reserved', () => {
    const actions = mcpConnectionActions(connection({ discoveredTools: PACK_TOOLS }))
    expect(actions.map((a) => a.type)).toEqual([
      MANIFEST_TOOL,
      PREFLIGHT_TOOL,
      pollToolName('tick'),
      'echo'
    ])
  })

  it('falls back to the tool name when a server offers no title', () => {
    expect(mcpToolToConnectorAction({ name: 'run_query' }).label).toBe('run_query')
    expect(mcpToolToConnectorAction({ name: 'run_query', title: '  ' }).label).toBe('run_query')
  })

  it('is empty until discovery has run', () => {
    expect(mcpConnectionActions(connection({}))).toEqual([])
  })
})
