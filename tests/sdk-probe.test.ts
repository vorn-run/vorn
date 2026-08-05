import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const transportInstances: MockTransport[] = []
const clientConnect = vi.fn()
const clientClose = vi.fn()
const listTools = vi.fn()
const callTool = vi.fn()

class MockTransport {
  readonly opts: Record<string, unknown>
  closed = false

  constructor(opts: Record<string, unknown>) {
    this.opts = opts
    transportInstances.push(this)
  }

  async close(): Promise<void> {
    this.closed = true
  }
}

vi.mock('@modelcontextprotocol/sdk/client/index.js', () => ({
  Client: class {
    connect = clientConnect
    close = clientClose
    listTools = listTools
    callTool = callTool
  }
}))

vi.mock('@modelcontextprotocol/sdk/client/stdio.js', () => ({
  StdioClientTransport: MockTransport
}))

const importProbe = async () => await import('../packages/server/src/connectors/sdk-probe')

beforeEach(() => {
  transportInstances.length = 0
  clientConnect.mockReset().mockResolvedValue(undefined)
  clientClose.mockReset().mockResolvedValue(undefined)
  listTools.mockReset().mockResolvedValue({ tools: [{ name: 'vorn_connector_manifest' }] })
  callTool.mockReset()
})

/** A manifest shaped the way `connectorManifest()` in the SDK emits one. */
function manifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'kusto',
    name: 'Azure Data Explorer',
    version: '0.1.0',
    description: 'Trigger from a KQL query',
    triggers: [
      {
        type: 'queryResult',
        label: 'Query result',
        description: 'Each new row',
        setup: {
          filters: {
            pollTool: 'poll_queryResult',
            itemsPath: 'items',
            idField: 'externalId',
            timestampField: 'updatedAt',
            titleField: 'title',
            urlField: 'url',
            cursorArg: 'cursor',
            cursorPath: 'nextCursor'
          },
          env: [
            { name: 'KUSTO_CLUSTER', required: true, secret: false, description: 'Cluster URL' },
            { name: 'KUSTO_TOKEN', required: false, secret: true }
          ]
        }
      }
    ],
    actions: [{ type: 'runQuery', label: 'Run query', description: 'Run KQL' }],
    ...overrides
  }
}

const respond = (payload: unknown): void => {
  callTool.mockResolvedValue({
    structuredContent: payload,
    content: [{ type: 'text', text: JSON.stringify(payload) }]
  })
}

describe('probeSdkConnector', () => {
  it('reads a manifest and reports the connector, its triggers and its actions', async () => {
    respond(manifest())
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: ['-y', 'pkg'] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.id).toBe('kusto')
    expect(result.manifest.name).toBe('Azure Data Explorer')
    expect(result.manifest.version).toBe('0.1.0')
    expect(result.manifest.triggers).toHaveLength(1)
    expect(result.manifest.triggers[0].filters.pollTool).toBe('poll_queryResult')
    expect(result.manifest.actions[0].type).toBe('runQuery')
  })

  it('rejects a blank command without spawning anything', async () => {
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: '   ' })

    expect(result).toEqual({ ok: false, error: 'A command is required' })
    expect(transportInstances).toHaveLength(0)
  })

  it('passes the request env to the child on top of a sanitized base', async () => {
    respond(manifest())
    const { probeSdkConnector } = await importProbe()

    await probeSdkConnector({ command: 'npx', args: ['-y', 'pkg'], env: { KUSTO_CLUSTER: 'c' } })

    const env = transportInstances[0].opts.env as Record<string, string>
    expect(env.KUSTO_CLUSTER).toBe('c')
  })

  it('explains itself when the server is a plain MCP server with no manifest tool', async () => {
    listTools.mockResolvedValue({ tools: [{ name: 'search' }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('does not describe itself')
    expect(callTool).not.toHaveBeenCalled()
  })

  it('surfaces the error text when the manifest tool itself fails', async () => {
    callTool.mockResolvedValue({ isError: true, content: [{ type: 'text', text: 'boom' }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result).toEqual({ ok: false, error: 'boom' })
  })

  it('falls back to the text block for a server that sends no structuredContent', async () => {
    callTool.mockResolvedValue({ content: [{ type: 'text', text: JSON.stringify(manifest()) }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.id).toBe('kusto')
  })

  it('reports a manifest that is missing an id rather than rendering a nameless connector', async () => {
    respond({ ...manifest(), id: '  ' })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result).toEqual({ ok: false, error: 'Connector manifest is missing an id or a name' })
  })

  it('reports a connector that offers nothing to connect to', async () => {
    respond({ ...manifest(), triggers: [], actions: [] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('no triggers and no actions')
  })

  it('returns a message rather than throwing when the payload is not JSON at all', async () => {
    callTool.mockResolvedValue({ content: [{ type: 'text', text: 'not json' }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result).toEqual({ ok: false, error: 'vorn_connector_manifest returned no manifest' })
  })

  it('collects the union of env across triggers, keeping the first description of each', async () => {
    respond(
      manifest({
        triggers: [
          {
            type: 'a',
            label: 'A',
            setup: { filters: {}, env: [{ name: 'SHARED', required: true, description: 'first' }] }
          },
          {
            type: 'b',
            label: 'B',
            setup: {
              filters: {},
              env: [
                { name: 'SHARED', required: false, description: 'second' },
                { name: 'ONLY_B', required: true, secret: true }
              ]
            }
          }
        ]
      })
    )
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.env).toEqual([
      { name: 'SHARED', required: true, secret: false, description: 'first' },
      { name: 'ONLY_B', required: true, secret: true }
    ])
  })

  it('defaults filter fields a trigger leaves out so the connection still polls', async () => {
    respond(manifest({ triggers: [{ type: 'items', label: 'Items', setup: { filters: {} } }] }))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.triggers[0].filters).toEqual({
      pollTool: 'poll_items',
      itemsPath: 'items',
      idField: 'externalId',
      timestampField: 'updatedAt',
      titleField: 'title',
      urlField: 'url',
      cursorArg: 'cursor',
      cursorPath: 'nextCursor'
    })
  })

  it('skips malformed trigger and env entries instead of failing the whole probe', async () => {
    respond(
      manifest({
        triggers: [
          null,
          { type: '   ', label: 'Blank' },
          { type: 'ok', label: 'Ok', setup: { env: ['nope', { name: '' }, { name: 'GOOD' }] } }
        ]
      })
    )
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.triggers.map((t) => t.type)).toEqual(['ok'])
    expect(result.manifest.env).toEqual([{ name: 'GOOD', required: false, secret: false }])
  })

  it('closes the child even when the probe times out, so nothing is left running', async () => {
    clientConnect.mockImplementation(() => new Promise(() => {}))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' }, { timeoutMs: 10 })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('Timed out')
    expect(clientClose).toHaveBeenCalled()
    expect(transportInstances[0].closed).toBe(true)
  })

  it('closes the child when the connector crashes on startup', async () => {
    clientConnect.mockRejectedValue(new Error('spawn failed'))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result).toEqual({ ok: false, error: 'spawn failed' })
    expect(transportInstances[0].closed).toBe(true)
  })

  it('still returns a result when closing the child throws', async () => {
    respond(manifest())
    clientClose.mockRejectedValue(new Error('already gone'))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
  })
})

describe('probeSdkConnector icon handling', () => {
  const withIcon = (icon: unknown) => respond({ ...manifest(), icon })

  it('passes through a well-formed icon', async () => {
    withIcon({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z', 'M8 8l2 2'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon).toEqual({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z', 'M8 8l2 2'] })
  })

  it('defaults the viewBox when the connector omits or malforms it', async () => {
    withIcon({ viewBox: 'not a viewbox', paths: ['M1 1h4v4z'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon?.viewBox).toBe('0 0 24 24')
  })

  it('drops an icon containing markup rather than path data', async () => {
    withIcon({ paths: ['M1 1h4v4z', '"/><script>alert(1)</script>'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    // The whole icon goes, not just the offending path — a partial glyph
    // would render as garbage.
    expect(result.manifest.icon).toBeUndefined()
  })

  it.each([
    ['no paths at all', { paths: [] }],
    ['a non-array paths', { paths: 'M1 1h4v4z' }],
    ['a non-string path', { paths: [42] }],
    ['not an object', 'M1 1h4v4z'],
    ['absurdly many paths', { paths: Array.from({ length: 25 }, () => 'M1 1h4v4z') }],
    ['an absurdly long path', { paths: ['M'.repeat(8_001)] }]
  ])('drops an icon with %s', async (_label, icon) => {
    withIcon(icon)
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon).toBeUndefined()
  })

  it('leaves the connector usable when its icon is rejected', async () => {
    withIcon({ paths: ['<svg/>'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx' })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.triggers).toHaveLength(1)
  })
})
