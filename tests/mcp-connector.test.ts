import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const getOrStartClientMock = vi.fn()
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: (...args: unknown[]) => getOrStartClientMock(...args),
  stopClient: vi.fn(),
  stopAllClients: vi.fn(),
  hasClient: vi.fn()
}))

const importMcp = async () => await import('../packages/server/src/connectors/mcp')

beforeEach(() => {
  getOrStartClientMock.mockReset()
})

function conn(filters: Record<string, unknown> = {}): SourceConnection {
  return {
    id: 'c1',
    connectorId: 'mcp',
    name: 'Test MCP',
    filters,
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-04-24T00:00:00Z'
  }
}

describe('mcpToolToConnectorAction', () => {
  it('maps string properties to text fields with supportsTemplates', async () => {
    const { mcpToolToConnectorAction } = await importMcp()
    const action = mcpToolToConnectorAction({
      name: 'read_file',
      description: 'Reads a file',
      inputSchema: {
        type: 'object',
        properties: { path: { type: 'string', description: 'File path' } },
        required: ['path']
      }
    })
    expect(action.type).toBe('read_file')
    expect(action.label).toBe('read_file')
    expect(action.description).toBe('Reads a file')
    expect(action.configFields).toHaveLength(1)
    expect(action.configFields[0]).toMatchObject({
      key: 'path',
      type: 'text',
      required: true,
      description: 'File path',
      supportsTemplates: true
    })
  })

  it('maps enums to select fields with string options', async () => {
    const { mcpToolToConnectorAction } = await importMcp()
    const action = mcpToolToConnectorAction({
      name: 'pick',
      inputSchema: {
        type: 'object',
        properties: { mode: { enum: ['fast', 'slow'] } }
      }
    })
    const field = action.configFields[0]
    expect(field.type).toBe('select')
    expect(field.options).toEqual([
      { value: 'fast', label: 'fast' },
      { value: 'slow', label: 'slow' }
    ])
  })

  it('maps object/array properties to textareas with a JSON-shape hint', async () => {
    const { mcpToolToConnectorAction } = await importMcp()
    const action = mcpToolToConnectorAction({
      name: 'complex',
      inputSchema: {
        type: 'object',
        properties: {
          cfg: { type: 'object' },
          tags: { type: 'array' }
        }
      }
    })
    const cfg = action.configFields.find((f) => f.key === 'cfg')
    const tags = action.configFields.find((f) => f.key === 'tags')
    expect(cfg?.type).toBe('textarea')
    expect(cfg?.placeholder).toBe('{} or []')
    expect(tags?.type).toBe('textarea')
  })

  it('uses a property default as the placeholder for scalar fields', async () => {
    const { mcpToolToConnectorAction } = await importMcp()
    const action = mcpToolToConnectorAction({
      name: 'with_default',
      inputSchema: {
        type: 'object',
        properties: { name: { type: 'string', default: 'world' } }
      }
    })
    expect(action.configFields[0].placeholder).toBe('"world"')
  })

  it('passes the tool outputSchema through to the action def', async () => {
    const { mcpToolToConnectorAction } = await importMcp()
    const outputSchema = { type: 'object', properties: { hit: { type: 'boolean' } } }
    const action = mcpToolToConnectorAction({
      name: 'ping',
      inputSchema: { type: 'object', properties: {} },
      outputSchema
    })
    expect(action.outputSchema).toBe(outputSchema)
  })
})

describe('mcpConnectionActions', () => {
  it('returns [] when no tools have been discovered yet', async () => {
    const { mcpConnectionActions } = await importMcp()
    expect(mcpConnectionActions(conn())).toEqual([])
  })

  it('maps each discovered tool to a ConnectorActionDef', async () => {
    const { mcpConnectionActions } = await importMcp()
    const actions = mcpConnectionActions(
      conn({
        discoveredTools: [
          { name: 'a', inputSchema: { type: 'object', properties: {} } },
          { name: 'b', inputSchema: { type: 'object', properties: {} } }
        ]
      })
    )
    expect(actions.map((a) => a.type)).toEqual(['a', 'b'])
  })
})

describe('invokeMcpTool', () => {
  it('coerces string form values back to their declared types before calling the tool', async () => {
    const callTool = vi.fn().mockResolvedValue({ content: [] })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    await invokeMcpTool(
      conn({
        discoveredTools: [
          {
            name: 'do',
            inputSchema: {
              type: 'object',
              properties: {
                n: { type: 'number' },
                flag: { type: 'boolean' },
                obj: { type: 'object' }
              }
            }
          }
        ]
      }),
      'do',
      { n: '42', flag: 'true', obj: '{"k":1}' }
    )
    expect(callTool).toHaveBeenCalledWith({
      name: 'do',
      arguments: { n: 42, flag: true, obj: { k: 1 } }
    })
  })

  it('prefers structuredContent over the raw envelope as the action output', async () => {
    const callTool = vi
      .fn()
      .mockResolvedValue({ content: [{ type: 'text', text: 'ok' }], structuredContent: { a: 1 } })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    const result = await invokeMcpTool(conn(), 'do', {})
    expect(result.success).toBe(true)
    expect(result.output).toEqual({ a: 1 })
  })

  it('surfaces an MCP-reported error with a helpful message', async () => {
    const callTool = vi.fn().mockResolvedValue({
      isError: true,
      content: [{ type: 'text', text: 'path not allowed' }]
    })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    const result = await invokeMcpTool(conn(), 'read_file', { path: '/etc/passwd' })
    expect(result.success).toBe(false)
    expect(result.error).toBe('path not allowed')
  })

  it('drops empty-string optional non-string fields so servers can fall back to defaults', async () => {
    const callTool = vi.fn().mockResolvedValue({ content: [] })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    await invokeMcpTool(
      conn({
        discoveredTools: [
          {
            name: 'do',
            inputSchema: {
              type: 'object',
              properties: { maxCount: { type: 'number' } }
            }
          }
        ]
      }),
      'do',
      { maxCount: '' }
    )
    expect(callTool).toHaveBeenCalledWith({ name: 'do', arguments: {} })
  })

  it('preserves empty strings for string-typed fields', async () => {
    const callTool = vi.fn().mockResolvedValue({ content: [] })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    await invokeMcpTool(
      conn({
        discoveredTools: [
          {
            name: 'do',
            inputSchema: { type: 'object', properties: { search: { type: 'string' } } }
          }
        ]
      }),
      'do',
      { search: '' }
    )
    expect(callTool).toHaveBeenCalledWith({ name: 'do', arguments: { search: '' } })
  })

  it('preserves empty strings on required non-string fields so the MCP server surfaces its own validation error', async () => {
    const callTool = vi.fn().mockResolvedValue({ content: [] })
    getOrStartClientMock.mockResolvedValue({ callTool })
    const { invokeMcpTool } = await importMcp()
    await invokeMcpTool(
      conn({
        discoveredTools: [
          {
            name: 'do',
            inputSchema: {
              type: 'object',
              properties: { count: { type: 'number' } },
              required: ['count']
            }
          }
        ]
      }),
      'do',
      { count: '' }
    )
    expect(callTool).toHaveBeenCalledWith({ name: 'do', arguments: { count: '' } })
  })

  it('returns a failure result when the client itself throws', async () => {
    getOrStartClientMock.mockRejectedValue(new Error('spawn failed'))
    const { invokeMcpTool } = await importMcp()
    const result = await invokeMcpTool(conn(), 'anything', {})
    expect(result).toEqual({ success: false, error: 'spawn failed' })
  })
})

describe('discoverTools', () => {
  it('calls listTools on the cached client and normalizes the result', async () => {
    const listTools = vi.fn().mockResolvedValue({
      tools: [{ name: 't1', description: 'd1', inputSchema: { type: 'object' } }, { name: 't2' }]
    })
    getOrStartClientMock.mockResolvedValue({ listTools })
    const { discoverTools } = await importMcp()
    const tools = await discoverTools(conn())
    expect(tools).toEqual([
      { name: 't1', description: 'd1', inputSchema: { type: 'object' } },
      { name: 't2' }
    ])
  })
})

describe('mcpConnector.describe', () => {
  it('declares command, args, env, and secretEnv auth fields', async () => {
    const { mcpConnector } = await importMcp()
    const manifest = mcpConnector.describe()
    const keys = manifest.auth.map((f) => f.key)
    // The connection form leads with the server/credential fields (optional
    // poll-config fields follow — asserted separately).
    expect(keys.slice(0, 4)).toEqual(['command', 'args', 'env', 'secretEnv'])
    expect(manifest.auth.find((f) => f.key === 'secretEnv')?.type).toBe('password')
  })

  it('exposes actions and triggers, with an empty static action list', async () => {
    const { mcpConnector } = await importMcp()
    expect(mcpConnector.capabilities).toEqual(['actions', 'triggers'])
    // Actions are per-connection (discovered via tools/list), so the static
    // list stays empty.
    expect(mcpConnector.describe().actions).toEqual([])
  })

  it('exposes a single mcpPoll trigger driven by the connection poll config', async () => {
    const { mcpConnector, MCP_POLL_EVENT } = await importMcp()
    const triggers = mcpConnector.describe().triggers ?? []
    expect(triggers).toHaveLength(1)
    expect(triggers[0].type).toBe(MCP_POLL_EVENT)
    // The poll mapping lives on the connection (auth/config form), not the
    // trigger, so the trigger itself declares no config fields.
    expect(triggers[0].configFields).toEqual([])
  })

  it('advertises the optional poll-config fields on the connection form', async () => {
    const { mcpConnector } = await importMcp()
    const keys = mcpConnector.describe().auth.map((f) => f.key)
    for (const k of ['pollTool', 'itemsPath', 'idField', 'timestampField']) {
      expect(keys).toContain(k)
    }
    // Poll fields are optional so action-only connections aren't forced to set them.
    expect(mcpConnector.describe().auth.find((f) => f.key === 'pollTool')?.required).toBeFalsy()
  })
})
