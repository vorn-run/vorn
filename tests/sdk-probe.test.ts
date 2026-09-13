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

  const browser = {
    rung: 'browser',
    browser: {
      signInUrl: 'https://substack.com/sign-in',
      origins: ['https://substack.com', 'https://*.substack.com'],
      check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name', 'handle'] }
    }
  }

  it('carries a browser sign-in whole, so the app knows where to sign in and what to check', async () => {
    expect(await probeAuth(browser)).toEqual(browser)
  })

  it('drops a browser sign-in whose pages sit outside its own origins', async () => {
    expect(await probeAuth({ rung: 'browser' })).toBeUndefined()
    const elsewhere = { ...browser.browser, signInUrl: 'https://evil.io/login' }
    expect(await probeAuth({ ...browser, browser: elsewhere })).toBeUndefined()
    const plainHttp = { ...browser.browser, origins: ['http://substack.com'] }
    expect(await probeAuth({ ...browser, browser: plainHttp })).toBeUndefined()
  })

  it('keeps the headers a check declares, and drops any a connector may not set', async () => {
    const headers = { 'X-CSRF-Protection': '1', Cookie: 'sid=1' }
    const declared = { ...browser.browser, check: { ...browser.browser.check, headers } }
    const auth = await probeAuth({ ...browser, browser: declared })
    expect(auth?.browser?.check.headers).toEqual({ 'X-CSRF-Protection': '1' })
  })

  it('leaves headers out when none of them may be sent', async () => {
    const declared = {
      ...browser.browser,
      check: { ...browser.browser.check, headers: { Cookie: 'sid=1' } }
    }
    expect(await probeAuth({ ...browser, browser: declared })).toEqual(browser)
  })

  it('carries what to borrow, since the token is fetched fresh at spawn', async () => {
    const auth = { ...cli, borrow: { env: ['GITLAB_HOST'], tokenArgs: ['auth', 'token'] } }
    expect(await probeAuth(auth)).toEqual(auth)
  })

  it('names the token variable by the case it was declared in, however it was asked for', async () => {
    const auth = {
      ...cli,
      borrow: {
        env: ['GITLAB_TOKEN', 'GITLAB_HOST'],
        tokenArgs: ['auth', 'token'],
        tokenEnv: ' gitlab_token '
      }
    }
    expect((await probeAuth(auth))?.borrow?.tokenEnv).toBe('GITLAB_TOKEN')
    const stranger = { ...auth, borrow: { ...auth.borrow, tokenEnv: 'OTHER' } }
    expect((await probeAuth(stranger))?.borrow?.tokenEnv).toBeUndefined()
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

describe('what a probed action returns', () => {
  const probeOutputs = async (outputs: unknown) => {
    const { probeSdkConnector } = await importProbe()
    respond(manifest({ actions: [{ type: 'post', label: 'Post', outputs }] }))
    const result = await probeSdkConnector({ command: 'npx', args: [] })
    if (!result.ok) throw new Error(result.error)
    return result.manifest.actions[0].outputs
  }

  it('carries the fields a step can read back', async () => {
    const outputs = [{ key: 'id', type: 'string', description: 'The new id' }]
    expect(await probeOutputs(outputs)).toEqual(outputs)
  })

  it('drops an output with no key and keeps only string facts', async () => {
    expect(await probeOutputs([{ type: 'string' }, 'id', { key: 'ok', type: 7 }])).toEqual([
      { key: 'ok' }
    ])
  })
})

describe('what a probed extension contributes', () => {
  const extensionManifest = (overrides: Record<string, unknown> = {}) => ({
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    permissions: ['terminal.read'],
    activates: { workspaceContains: ['package.json'] },
    contributes: {
      footers: [{ id: 'checks', title: 'Checks', every: 30 }],
      panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }],
      linkHandlers: [{ id: 'pr', title: 'Pull request', pattern: 'github\\.com' }]
    },
    ...overrides
  })

  const probeExtension = async (overrides: Record<string, unknown> = {}) => {
    const { probeSdkConnector } = await importProbe()
    callTool.mockResolvedValue({ structuredContent: extensionManifest(overrides) })
    return probeSdkConnector({ command: 'npx', args: [] })
  }

  // A glyph is decoration on a row that still has a name; refusing the pane over
  // it would cost the person the pane rather than the picture.
  it('keeps a pane whose glyph it cannot draw, and the glyph it can', async () => {
    const result = await probeExtension({
      contributes: {
        panes: [
          {
            id: 'report',
            title: 'Report',
            web: 'web/report/index.html',
            icon: { paths: ['<svg>'] }
          },
          {
            id: 'top',
            title: 'Top',
            command: ['top'],
            icon: { viewBox: '0 0 24 24', paths: ['M4 4h16v16H4z'] }
          }
        ]
      }
    })
    if (!result.ok) throw new Error(result.error)

    const panes = result.manifest.contributes?.panes ?? []
    expect(panes.map((pane) => pane.id)).toEqual(['report', 'top'])
    expect(panes[0].icon).toBeUndefined()
    expect(panes[1].icon).toEqual({ viewBox: '0 0 24 24', paths: ['M4 4h16v16H4z'] })
  })

  it('reads an extension that has no triggers and no actions', async () => {
    const result = await probeExtension()
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.kind).toBe('extension')
    expect(result.manifest.triggers).toEqual([])
    expect(result.manifest.actions).toEqual([])
    expect(result.manifest.permissions).toEqual(['terminal.read'])
    expect(result.manifest.activates).toEqual({ workspaceContains: ['package.json'] })
    expect(result.manifest.contributes?.footers?.[0]).toEqual({
      id: 'checks',
      title: 'Checks',
      every: 30
    })
  })

  it('refuses an extension that contributes nothing, the way a connector with no triggers is refused', async () => {
    const result = await probeExtension({ contributes: {} })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/contributes/)
  })

  it('drops a permission this build cannot enforce rather than granting it', async () => {
    const result = await probeExtension({
      permissions: ['terminal.read', 'filesystem.write', 'terminal.read']
    })
    if (!result.ok) throw new Error(result.error)
    expect(result.manifest.permissions).toEqual(['terminal.read'])
  })

  it('drops a contribution the app could not draw, and keeps the rest', async () => {
    const result = await probeExtension({
      contributes: {
        panes: [
          { id: 'escape', title: 'Escape', web: '../outside/index.html' },
          { id: 'absolute', title: 'Absolute', web: '/etc/passwd.html' },
          { id: 'empty', title: 'Empty', command: ['viewer', ''] },
          { id: 'report', title: 'Report', web: 'web/report/index.html' }
        ],
        footers: [
          { id: 'spin', title: 'Spin', every: 1 },
          { id: 'checks', title: 'Checks', every: 30 }
        ],
        linkHandlers: [
          { id: 'broken', title: 'Broken', pattern: '([' },
          { id: 'chewy', title: 'Chewy', pattern: '(a+)+$' },
          { id: 'pr', title: 'Pull request', pattern: 'github\\.com' }
        ]
      }
    })
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.contributes?.panes?.map((pane) => pane.id)).toEqual(['report'])
    expect(result.manifest.contributes?.footers?.map((footer) => footer.id)).toEqual(['checks'])
    expect(result.manifest.contributes?.linkHandlers?.map((entry) => entry.id)).toEqual(['pr'])
  })

  it('drops an agent or a platform this build cannot evaluate', async () => {
    const result = await probeExtension({
      activates: {
        agent: ['claude', '<img src=x onerror=1>', 'nope'],
        platform: ['darwin', 'plan9'],
        remoteHost: ['github.com']
      }
    })
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.activates).toEqual({
      remoteHost: ['github.com'],
      agent: ['claude'],
      platform: ['darwin']
    })
  })

  it('trims a host name and drops one that is only whitespace', async () => {
    const result = await probeExtension({
      activates: { remoteHost: [' github.com ', '   ', ''] }
    })
    if (!result.ok) throw new Error(result.error)
    expect(result.manifest.activates).toEqual({ remoteHost: ['github.com'] })
  })

  it('keeps a predicate naming nothing this build knows from narrowing to nothing', async () => {
    const result = await probeExtension({ activates: { agent: ['emacs'], platform: ['plan9'] } })
    if (!result.ok) throw new Error(result.error)
    expect(result.manifest.activates).toBeUndefined()
  })

  it('holds what it reads to a length a card can draw', async () => {
    const result = await probeExtension({
      contributes: {
        footers: [
          { id: 'checks', title: 'T'.repeat(5_000), description: 'D'.repeat(5_000), every: 30 }
        ]
      }
    })
    if (!result.ok) throw new Error(result.error)

    const footer = result.manifest.contributes?.footers?.[0]
    expect(footer?.title.length).toBe(500)
    expect(footer?.description?.length).toBe(500)
  })

  it('reads at most a screenful of contributions of each kind', async () => {
    const many = (kind: 'panes' | 'footers') =>
      Array.from({ length: 50 }, (_, index) => ({
        id: `c${index}`,
        title: `C${index}`,
        ...(kind === 'panes' ? { web: 'web/report/index.html' } : { every: 30 })
      }))
    const result = await probeExtension({
      contributes: { panes: many('panes'), footers: many('footers') }
    })
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.contributes?.panes).toHaveLength(32)
    expect(result.manifest.contributes?.footers).toHaveLength(32)
  })

  it('drops a pattern too long to match on every click, and keeps the example of one that is not', async () => {
    const result = await probeExtension({
      contributes: {
        linkHandlers: [
          { id: 'long', title: 'Long', pattern: 'a'.repeat(257) },
          {
            id: 'pr',
            title: 'Pull request',
            pattern: 'github\\.com',
            example: 'https://github.com/vorn-run/vorn/pull/1'
          }
        ]
      }
    })
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.contributes?.linkHandlers).toEqual([
      {
        id: 'pr',
        title: 'Pull request',
        pattern: 'github\\.com',
        example: 'https://github.com/vorn-run/vorn/pull/1'
      }
    ])
  })

  it('reads a manifest with no kind as the connector it was written as', async () => {
    const { probeSdkConnector } = await importProbe()
    callTool.mockResolvedValue({ structuredContent: manifest() })
    const result = await probeSdkConnector({ command: 'npx', args: [] })
    if (!result.ok) throw new Error(result.error)

    expect(result.manifest.kind).toBe('connector')
    expect(result.manifest.contributes).toBeUndefined()
  })
})
