import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'

const rpcCall = vi.fn()
vi.mock('../packages/mcp/src/ws-client', () => ({ rpcCall: (...a: unknown[]) => rpcCall(...a) }))

const { registerConnectorTools } = await import('../packages/mcp/src/tools/connectors')

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

/** Collects the tools the module registers so they can be invoked directly. */
function collect(): Map<string, Handler> {
  const tools = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      tools.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerConnectorTools(server)
  return tools
}

const MANIFEST = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  version: '0.5.2',
  triggers: [
    { type: 'queryResult', label: 'Query result', filters: { pollTool: 'poll_queryResult' } }
  ],
  actions: [],
  env: [
    { name: 'KUSTO_CLUSTER', required: true, secret: false, description: 'Cluster URL' },
    { name: 'KUSTO_LOOKBACK', required: false, secret: false }
  ]
}

const CATALOG = [
  {
    id: 'kusto',
    name: 'Azure Data Explorer',
    description: 'Trigger from a KQL query',
    packageName: '@vornrun/connector-kusto',
    capabilities: ['triggers'],
    launch: { command: 'npx', args: ['-y', '@vornrun/connector-kusto'] }
  }
]

/** Route each RPC method to a canned answer, so a tool can be driven end to end. */
function server(overrides: Record<string, unknown> = {}) {
  const defaults: Record<string, unknown> = {
    'connector:list': [
      { id: 'github', name: 'GitHub', capabilities: ['tasks'], manifest: {} },
      { id: 'mcp', name: 'MCP', capabilities: ['actions'], manifest: {} }
    ],
    'connector:catalog': CATALOG,
    'connection:list': [],
    'connector:status': [{ connectorId: 'github', authed: true }],
    'connector:probeSdk': { ok: true, manifest: MANIFEST },
    'connection:create': { id: 'new-conn' },
    'connection:listActions': [],
    'connection:executeAction': { success: true, data: {} },
    'connection:backfill': { imported: 0, updated: 0 }
  }
  const table = { ...defaults, ...overrides }
  rpcCall.mockImplementation((method: string) => Promise.resolve(table[method]))
}

const text = (r: { content: Array<{ text: string }> }) => r.content.map((c) => c.text).join('\n')
const parsed = (r: { content: Array<{ text: string }> }) => JSON.parse(text(r))

let tools: Map<string, Handler>

beforeEach(() => {
  rpcCall.mockReset()
  server()
  tools = collect()
})

describe('connector tools', () => {
  it('registers discovery, install and invocation tools', () => {
    expect([...tools.keys()]).toEqual([
      'list_connectors',
      'list_connections',
      'list_connector_actions',
      'inspect_connector_package',
      'install_connector',
      'run_connector_action',
      'backfill_connection'
    ])
  })
})

describe('list_connectors', () => {
  it('shows built-in and installable connectors in one list', async () => {
    const result = parsed(await tools.get('list_connectors')!({}))

    expect(result.map((c: { id: string }) => c.id)).toEqual(['github', 'mcp', 'kusto'])
    expect(result[2]).toMatchObject({ source: 'package', package: '@vornrun/connector-kusto' })
  })

  it('reports auth state, so an agent can explain why a connector fails', async () => {
    server({
      'connector:status': [{ connectorId: 'github', authed: false, message: 'run gh auth login' }]
    })

    const result = parsed(await tools.get('list_connectors')!({}))

    expect(result[0]).toMatchObject({ authenticated: false, authMessage: 'run gh auth login' })
  })

  it('counts a packaged connection against its own connector, not mcp', async () => {
    // Every packaged connector is stored as `mcp`, so counting by connectorId
    // would credit all of them to MCP and none to themselves.
    server({
      'connection:list': [{ id: 'c1', connectorId: 'mcp', filters: { sdkConnectorId: 'kusto' } }]
    })

    const result = parsed(await tools.get('list_connectors')!({}))

    expect(result.find((c: { id: string }) => c.id === 'kusto').connections).toBe(1)
    expect(result.find((c: { id: string }) => c.id === 'mcp').connections).toBe(0)
  })

  it('narrows to what is not set up yet when asked', async () => {
    server({
      'connection:list': [{ id: 'c1', connectorId: 'mcp', filters: { sdkConnectorId: 'kusto' } }]
    })

    const result = parsed(await tools.get('list_connectors')!({ installable_only: true }))

    expect(result.map((c: { id: string }) => c.id)).not.toContain('kusto')
  })
})

describe('list_connections', () => {
  const CONNECTIONS = [
    {
      id: 'c1',
      name: 'repo',
      connectorId: 'github',
      filters: { owner: 'me' },
      lastSyncError: 'Validation Failed (HTTP 422)'
    },
    { id: 'c2', name: 'other', connectorId: 'linear', filters: {} }
  ]

  it('surfaces the last sync error, which is the point of asking', async () => {
    server({ 'connection:list': CONNECTIONS })

    const result = parsed(await tools.get('list_connections')!({}))

    expect(result[0].lastSyncError).toBe('Validation Failed (HTTP 422)')
  })

  it('narrows to failing connections', async () => {
    server({ 'connection:list': CONNECTIONS })

    const result = parsed(await tools.get('list_connections')!({ failing_only: true }))

    expect(result.map((c: { id: string }) => c.id)).toEqual(['c1'])
  })

  it('filters by connector using the packaged id, not the stored mcp id', async () => {
    server({
      'connection:list': [
        { id: 'c1', name: 'k', connectorId: 'mcp', filters: { sdkConnectorId: 'kusto' } }
      ]
    })

    const result = parsed(await tools.get('list_connections')!({ connector_id: 'kusto' }))

    expect(result).toHaveLength(1)
  })

  it('never returns stored credentials', async () => {
    server({
      'connection:list': [
        {
          id: 'c1',
          name: 'k',
          connectorId: 'mcp',
          filters: { env: '{}', secretEnv: 'ENCRYPTED-BLOB', discoveredTools: [{ name: 'x' }] }
        }
      ]
    })

    const out = text(await tools.get('list_connections')!({}))

    expect(out).not.toContain('ENCRYPTED-BLOB')
    expect(out).not.toContain('secretEnv')
  })
})

describe('inspect_connector_package', () => {
  it('reads a package without creating anything', async () => {
    const result = parsed(await tools.get('inspect_connector_package')!({ package: 'pkg' }))

    expect(result.name).toBe('Azure Data Explorer')
    expect(rpcCall).not.toHaveBeenCalledWith(
      'connection:create',
      expect.anything(),
      expect.anything()
    )
  })

  it('runs a bare package name through npx and a command as given', async () => {
    await tools.get('inspect_connector_package')!({ package: '@vornrun/connector-kusto' })
    expect(rpcCall).toHaveBeenCalledWith(
      'connector:probeSdk',
      { command: 'npx', args: ['-y', '@vornrun/connector-kusto'] },
      expect.any(Number)
    )

    await tools.get('inspect_connector_package')!({ package: 'node /tmp/dist/index.js' })
    expect(rpcCall).toHaveBeenCalledWith(
      'connector:probeSdk',
      { command: 'node', args: ['/tmp/dist/index.js'] },
      expect.any(Number)
    )
  })

  it('allows longer than a normal call, since the package downloads first', async () => {
    await tools.get('inspect_connector_package')!({ package: 'pkg' })

    const [, , timeout] = rpcCall.mock.calls.find((c) => c[0] === 'connector:probeSdk')!
    expect(timeout).toBeGreaterThan(10_000)
  })

  it('reports why a package could not be read', async () => {
    server({ 'connector:probeSdk': { ok: false, error: 'package not found' } })

    const result = await tools.get('inspect_connector_package')!({ package: 'nope' })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('package not found')
  })
})

describe('install_connector', () => {
  const ENV = { KUSTO_CLUSTER: 'https://help.kusto.windows.net' }

  it('installs a catalog connector and reports the connection', async () => {
    const result = parsed(
      await tools.get('install_connector')!({ connector_id: 'kusto', env: ENV })
    )

    expect(result.connectionId).toBe('new-conn')
    expect(result.trigger).toBe('queryResult')
  })

  it('records what it installed so the connection can be identified later', async () => {
    await tools.get('install_connector')!({ connector_id: 'kusto', env: ENV })

    const [, params] = rpcCall.mock.calls.find((c) => c[0] === 'connection:create')!
    expect(params.filters).toMatchObject({
      sdkConnectorId: 'kusto',
      sdkVersion: '0.5.2',
      env: JSON.stringify(ENV),
      pollTool: 'poll_queryResult'
    })
  })

  it('names the catalog ids it knows when given one it does not', async () => {
    const result = await tools.get('install_connector')!({ connector_id: 'nope' })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('kusto')
  })

  it('refuses without a connector to install rather than guessing', async () => {
    const result = await tools.get('install_connector')!({})

    expect(result.isError).toBe(true)
    expect(rpcCall).not.toHaveBeenCalledWith('connection:create', expect.anything())
  })

  it('lists the missing values instead of creating a connection that cannot run', async () => {
    const result = await tools.get('install_connector')!({ connector_id: 'kusto' })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('KUSTO_CLUSTER')
    expect(text(result)).toContain('Cluster URL')
    expect(rpcCall).not.toHaveBeenCalledWith('connection:create', expect.anything())
  })

  it('rejects an env var the connector does not use, which is usually a typo', async () => {
    const result = await tools.get('install_connector')!({
      connector_id: 'kusto',
      env: { ...ENV, KUSTO_CLUSTR: 'x' }
    })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('KUSTO_CLUSTR')
  })

  it('refuses to store a secret, which it cannot encrypt from here', async () => {
    // Encryption runs in the desktop process; accepting the value here would
    // mean writing a credential to the database in the clear.
    server({
      'connector:probeSdk': {
        ok: true,
        manifest: {
          ...MANIFEST,
          env: [{ name: 'API_TOKEN', required: true, secret: true }]
        }
      }
    })

    const result = await tools.get('install_connector')!({
      connector_id: 'kusto',
      env: { API_TOKEN: 'super-secret' }
    })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('API_TOKEN')
    expect(text(result)).toMatch(/keychain|person/i)
    expect(rpcCall).not.toHaveBeenCalledWith('connection:create', expect.anything())
  })

  it('names the triggers on offer when asked for one that does not exist', async () => {
    const result = await tools.get('install_connector')!({
      connector_id: 'kusto',
      env: ENV,
      trigger: 'nope'
    })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('queryResult')
  })

  it('installs a package that is not in the catalog', async () => {
    const result = parsed(
      await tools.get('install_connector')!({ package: '@scope/other', env: ENV })
    )

    expect(result.connectionId).toBe('new-conn')
    const [, params] = rpcCall.mock.calls.find((c) => c[0] === 'connection:create')!
    expect(params.filters.command).toBe('npx')
  })

  it('reports a package that could not be started rather than saving it', async () => {
    server({ 'connector:probeSdk': { ok: false, error: 'no such package' } })

    const result = await tools.get('install_connector')!({ package: 'nope' })

    expect(result.isError).toBe(true)
    expect(rpcCall).not.toHaveBeenCalledWith('connection:create', expect.anything())
  })
})

describe('run_connector_action', () => {
  it('returns what the action produced', async () => {
    server({ 'connection:executeAction': { success: true, data: { rows: 2 } } })

    const result = parsed(
      await tools.get('run_connector_action')!({
        connection_id: 'c1',
        action: 'runQuery'
      })
    )

    expect(result.data.rows).toBe(2)
  })

  it('reports a failed action as an error, not as a result', async () => {
    server({ 'connection:executeAction': { success: false, error: 'query is invalid' } })

    const result = await tools.get('run_connector_action')!({ connection_id: 'c1', action: 'x' })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('query is invalid')
  })
})

describe('backfill_connection', () => {
  it('reports what it imported', async () => {
    server({ 'connection:backfill': { imported: 3, updated: 1 } })

    const result = parsed(await tools.get('backfill_connection')!({ connection_id: 'c1' }))

    expect(result).toMatchObject({ imported: 3, updated: 1 })
  })

  it('surfaces a backfill error rather than reporting zero imported', async () => {
    server({ 'connection:backfill': { imported: 0, updated: 0, error: 'Connection not found' } })

    const result = await tools.get('backfill_connection')!({ connection_id: 'nope' })

    expect(result.isError).toBe(true)
    expect(text(result)).toContain('Connection not found')
  })
})

describe('list_connector_actions', () => {
  it('returns the actions with their inputs', async () => {
    server({
      'connection:listActions': [{ type: 'runQuery', label: 'Run query', configFields: [] }]
    })

    const result = parsed(await tools.get('list_connector_actions')!({ connection_id: 'c1' }))

    expect(result[0].type).toBe('runQuery')
  })

  it('explains an empty list rather than returning a bare []', async () => {
    const result = await tools.get('list_connector_actions')!({ connection_id: 'c1' })

    expect(result.isError).toBe(true)
    expect(text(result)).toMatch(/discovery/i)
  })
})
