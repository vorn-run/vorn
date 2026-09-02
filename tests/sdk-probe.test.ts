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

    const result = await probeSdkConnector({ command: '   ', args: [] })

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

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('does not describe itself')
    expect(callTool).not.toHaveBeenCalled()
  })

  it('surfaces the error text when the manifest tool itself fails', async () => {
    callTool.mockResolvedValue({ isError: true, content: [{ type: 'text', text: 'boom' }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result).toEqual({ ok: false, error: 'boom' })
  })

  it('falls back to the text block for a server that sends no structuredContent', async () => {
    callTool.mockResolvedValue({ content: [{ type: 'text', text: JSON.stringify(manifest()) }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.id).toBe('kusto')
  })

  it('reports a manifest that is missing an id rather than rendering a nameless connector', async () => {
    respond({ ...manifest(), id: '  ' })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result).toEqual({ ok: false, error: 'Connector manifest is missing an id or a name' })
  })

  it('reports a connector that offers nothing to connect to', async () => {
    respond({ ...manifest(), triggers: [], actions: [] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('no triggers and no actions')
  })

  it('returns a message rather than throwing when the payload is not JSON at all', async () => {
    callTool.mockResolvedValue({ content: [{ type: 'text', text: 'not json' }] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

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

    const result = await probeSdkConnector({ command: 'npx', args: [] })

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

    const result = await probeSdkConnector({ command: 'npx', args: [] })

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

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.triggers.map((t) => t.type)).toEqual(['ok'])
    expect(result.manifest.env).toEqual([{ name: 'GOOD', required: false, secret: false }])
  })

  it('closes the child even when the probe times out, so nothing is left running', async () => {
    clientConnect.mockImplementation(() => new Promise(() => {}))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] }, { timeoutMs: 10 })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('Timed out')
    expect(clientClose).toHaveBeenCalled()
    expect(transportInstances[0].closed).toBe(true)
  })

  it('closes the child when the connector crashes on startup', async () => {
    clientConnect.mockRejectedValue(new Error('spawn failed'))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result).toEqual({ ok: false, error: 'spawn failed' })
    expect(transportInstances[0].closed).toBe(true)
  })

  it('still returns a result when closing the child throws', async () => {
    respond(manifest())
    clientClose.mockRejectedValue(new Error('already gone'))
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
  })
})

describe('probeSdkConnector icon handling', () => {
  const withIcon = (icon: unknown) => respond({ ...manifest(), icon })

  it('passes through a well-formed icon', async () => {
    withIcon({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z', 'M8 8l2 2'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon).toEqual({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z', 'M8 8l2 2'] })
  })

  it('defaults the viewBox when the connector omits or malforms it', async () => {
    withIcon({ viewBox: 'not a viewbox', paths: ['M1 1h4v4z'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon?.viewBox).toBe('0 0 24 24')
  })

  it('drops an icon containing markup rather than path data', async () => {
    withIcon({ paths: ['M1 1h4v4z', '"/><script>alert(1)</script>'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

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

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.icon).toBeUndefined()
  })

  it('leaves the connector usable when its icon is rejected', async () => {
    withIcon({ paths: ['<svg/>'] })
    const { probeSdkConnector } = await importProbe()

    const result = await probeSdkConnector({ command: 'npx', args: [] })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.manifest.triggers).toHaveLength(1)
  })
})

describe('what the probe accepts from a package', () => {
  const probeWith = async (trigger: Record<string, unknown>) => {
    const { probeSdkConnector } = await importProbe()
    callTool.mockResolvedValue({
      structuredContent: manifest({ triggers: [{ ...baseTrigger, ...trigger }] })
    })
    const result = await probeSdkConnector({ command: 'npx', args: [] })
    if (!result.ok) throw new Error(result.error)
    return result.manifest.triggers[0]
  }

  const baseTrigger = {
    type: 'queryResult',
    label: 'Query result',
    setup: { filters: {}, env: [] }
  }

  it('reads a status mapping the connector suggested', async () => {
    const trigger = await probeWith({
      statusMapping: [
        { upstream: 'open', suggestedLocal: 'todo' },
        { upstream: 'closed', suggestedLocal: 'done' }
      ]
    })
    expect(trigger.statusMapping).toEqual([
      { upstream: 'open', suggestedLocal: 'todo' },
      { upstream: 'closed', suggestedLocal: 'done' }
    ])
  })

  it('drops a local status it does not recognise', async () => {
    // This arrives from a third-party package. An unknown status written onto
    // a connection would fail a constraint later, far from the connector that
    // supplied it.
    const trigger = await probeWith({
      statusMapping: [
        { upstream: 'open', suggestedLocal: 'todo' },
        { upstream: 'weird', suggestedLocal: 'obliterated' }
      ]
    })
    expect(trigger.statusMapping).toEqual([{ upstream: 'open', suggestedLocal: 'todo' }])
  })

  it('ignores a mapping that is not a list at all', async () => {
    expect((await probeWith({ statusMapping: 'todo' })).statusMapping).toBeUndefined()
    expect((await probeWith({ statusMapping: [] })).statusMapping).toBeUndefined()
  })

  it('reads a polling workflow', async () => {
    const trigger = await probeWith({
      defaultWorkflow: { name: 'Kusto: rows', defaultCronFromMinutes: 10 }
    })
    expect(trigger.defaultWorkflow).toEqual({ name: 'Kusto: rows', defaultCronFromMinutes: 10 })
  })

  it('refuses an interval that would never fire or never stop', async () => {
    // A zero or fractional interval produces a cron that does one or the
    // other, and neither is worth guessing a correction for.
    for (const minutes of [0, -5, 1.5, 5000]) {
      const trigger = await probeWith({
        defaultWorkflow: { name: 'x', defaultCronFromMinutes: minutes }
      })
      expect(trigger.defaultWorkflow).toBeUndefined()
    }
  })

  it('refuses a workflow with no name', async () => {
    const trigger = await probeWith({ defaultWorkflow: { defaultCronFromMinutes: 5 } })
    expect(trigger.defaultWorkflow).toBeUndefined()
  })
})

describe('how a probed connector says it signs in', () => {
  const probeAuth = async (auth: unknown) => {
    const { probeSdkConnector } = await importProbe()
    respond(manifest({ auth }))
    const result = await probeSdkConnector({ command: 'npx', args: [] })
    if (!result.ok) throw new Error(result.error)
    return result.manifest.auth
  }

  const cli = { rung: 'cli', probe: { command: 'glab', args: ['auth', 'status'] } }

  it('carries a rung this build can act on, whole', async () => {
    expect(await probeAuth(cli)).toEqual(cli)
    expect(await probeAuth({ rung: 'none' })).toEqual({ rung: 'none' })
    expect(await probeAuth({ rung: 'key', keys: ['apiToken'] })).toEqual({
      rung: 'key',
      keys: ['apiToken']
    })
  })

  it('carries what to borrow, since the token is fetched fresh at spawn', async () => {
    const auth = { ...cli, borrow: { env: ['GITLAB_HOST'], tokenArgs: ['auth', 'token'] } }
    expect(await probeAuth(auth)).toEqual(auth)
  })

  it('says nothing rather than name a rung it cannot describe', async () => {
    expect(await probeAuth({ rung: 'sso' })).toBeUndefined()
    expect(await probeAuth({ rung: 7 })).toBeUndefined()
    expect(await probeAuth('cli')).toBeUndefined()
    expect(await probeAuth(undefined)).toBeUndefined()
  })

  it('drops a borrowed login whose probe could not be run', async () => {
    // The rung promises there is a command to ask who you are. Offering a
    // sign-in backed by nothing runnable is worse than offering none.
    expect(await probeAuth({ rung: 'cli' })).toBeUndefined()
    expect(await probeAuth({ rung: 'cli', probe: { command: '   ' } })).toBeUndefined()
    expect(await probeAuth({ rung: 'cli', probe: 'glab auth status' })).toBeUndefined()
  })

  it('refuses a probe command that is a path or carries shell syntax', async () => {
    // The host resolves a bare name on PATH and runs it without a shell, so
    // anything else was either a mistake or an attempt to run something else.
    for (const command of ['/usr/bin/glab', './glab', '../glab', 'glab; rm -rf /', 'glab && x']) {
      expect(await probeAuth({ rung: 'cli', probe: { command } })).toBeUndefined()
    }
  })

  it('refuses probe arguments that are not all strings', async () => {
    const stringy = { rung: 'cli', probe: { command: 'glab', args: 'status' } }
    const mixed = { rung: 'cli', probe: { command: 'glab', args: ['auth', 7] } }
    expect(await probeAuth(stringy)).toBeUndefined()
    expect(await probeAuth(mixed)).toBeUndefined()
  })

  it('keeps a key rung whose probe was unusable, minus the probe', async () => {
    // Only `cli` promises a runnable probe; a key rung still knows what it needs.
    const keyed = { rung: 'key', keys: ['apiToken'], probe: { command: '/bin/x' } }
    expect(await probeAuth(keyed)).toEqual({ rung: 'key', keys: ['apiToken'] })
  })
})

describe('what a probed action takes', () => {
  const probeInputs = async (inputs: unknown) => {
    const { probeSdkConnector } = await importProbe()
    respond(manifest({ actions: [{ type: 'post', label: 'Post', inputs }] }))
    const result = await probeSdkConnector({ command: 'npx', args: [] })
    if (!result.ok) throw new Error(result.error)
    return result.manifest.actions[0].inputs
  }

  it('carries the arguments a step will ask for', async () => {
    const inputs = [{ key: 'text', label: 'Text', type: 'string', required: true }]
    expect(await probeInputs(inputs)).toEqual(inputs)
  })

  it('carries a select whole, choices and options set alike', async () => {
    const inputs = [
      {
        key: 'reason',
        label: 'Reason',
        type: 'select',
        required: false,
        options: [{ value: 'fixed' }, { value: 'wontfix', label: 'Will not fix' }],
        loadOptions: 'reasons'
      }
    ]
    expect(await probeInputs(inputs)).toEqual(inputs)
  })

  it('fills in what a terse connector left out', async () => {
    expect(await probeInputs([{ key: 'text' }])).toEqual([
      { key: 'text', label: 'text', type: 'string', required: false }
    ])
  })

  it('drops an argument with no key, and a choice that selects nothing', async () => {
    expect(await probeInputs([{ label: 'Nameless' }, 'text', []])).toEqual([])
    const options = [{ label: 'Empty' }, 'high', { value: 'ok' }]
    expect((await probeInputs([{ key: 'v', options }]))?.[0].options).toEqual([{ value: 'ok' }])
    expect((await probeInputs([{ key: 'v', options: 'high,low' }]))?.[0].options).toBeUndefined()
  })
})
