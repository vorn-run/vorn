import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  ActionArgumentError,
  SessionUnavailableError,
  UpstreamStatusError,
  connectionSetup,
  connectorManifest,
  createConnectorServer,
  defineConnector,
  protocolError
} from '../packages/connector-sdk/src/index'
import { isEntryPoint, runCli } from '../packages/connector-sdk/src/cli'
import type { Connector } from '../packages/connector-sdk/src/types'
import { greeted } from './helpers/connector-server'

const NOW = '2026-08-05T00:00:00.000Z'
const HELLO = { protocols: [1], host: { name: 'vorn', version: 'test' } }

const connector: Connector = defineConnector({
  id: 'acme',
  name: 'Acme',
  version: '1.2.3',
  description: 'Acme tickets',
  config: [
    { key: 'apiToken', label: 'API token', required: true, secret: true },
    { key: 'orgUrl', label: 'Org URL', description: 'Base URL' }
  ],
  triggers: [
    {
      type: 'newTicket',
      label: 'New ticket',
      description: 'Tickets opened since the last poll',
      poll: (context) => ({
        items: [
          {
            externalId: '1',
            title: 'Ticket 1',
            updatedAt: NOW,
            data: { since: context.since ?? null, limit: context.limit ?? null }
          }
        ]
      })
    },
    {
      type: 'brokenTrigger',
      label: 'Broken',
      poll: () => {
        throw new Error('upstream exploded')
      }
    }
  ],
  actions: [
    {
      type: 'closeTicket',
      label: 'Close ticket',
      idempotent: true,
      inputs: [
        { key: 'id', label: 'Id', required: true },
        { key: 'reason', label: 'Reason', description: 'Why it was closed' }
      ],
      run: (args, context) => ({ closed: args.id, token: context.config.apiToken })
    }
  ]
})

const serve = () => greeted(connector, { config: { apiToken: 'tok' }, now: () => NOW })

afterEach(() => {
  vi.unstubAllEnvs()
})

describe('the hello a connector answers first', () => {
  it('names the protocol it speaks, the SDK it was built with, and itself', async () => {
    const server = createConnectorServer(connector, { config: {} })
    expect(
      await server.handle({ jsonrpc: '2.0', id: 1, method: 'vorn/hello', params: HELLO })
    ).toEqual({
      jsonrpc: '2.0',
      id: 1,
      result: {
        protocol: 1,
        sdk: { name: '@vornrun/connector-sdk', version: expect.any(String) },
        connector: { id: 'acme', version: '1.2.3', kind: 'connector' }
      }
    })
  })

  it('refuses a hello offering only protocols it does not speak, or none at all', async () => {
    const server = createConnectorServer(connector, { config: {} })
    const offered = (params: unknown) =>
      server.handle({ jsonrpc: '2.0', id: 1, method: 'vorn/hello', params })
    expect(await offered({ protocols: [2, 3], host: HELLO.host })).toMatchObject({
      error: { code: -32001, message: 'Vorn offers connector protocol 2, 3; acme speaks 1' }
    })
    expect(await offered({ host: HELLO.host })).toMatchObject({ error: { code: -32602 } })
  })

  it('answers nothing else before it', async () => {
    const server = createConnectorServer(connector, { config: {} })
    expect(
      await server.handle({ jsonrpc: '2.0', id: 1, method: 'connector/manifest', params: {} })
    ).toEqual({
      jsonrpc: '2.0',
      id: 1,
      error: { code: -32002, message: 'Call vorn/hello before connector/manifest' }
    })
  })

  it('answers no notification, and nothing that is not a request', async () => {
    const server = createConnectorServer(connector, { config: {} })
    expect(await server.handle({ jsonrpc: '2.0', method: 'vorn/hello', params: HELLO })).toBe(
      undefined
    )
    expect(await server.handle('vorn/hello')).toBe(undefined)
    expect(await server.handle([1, 2])).toBe(undefined)
    expect(await server.handle({ jsonrpc: '2.0', id: 'one', method: 'vorn/hello' })).toBe(undefined)
  })

  it('says so for a method it does not know, or a request that is not one', async () => {
    const server = await serve()
    expect(await server.fail('tools/list')).toEqual({ code: -32601, message: 'Method not found' })
    expect(await server.send('trigger/poll', 'newTicket')).toMatchObject({
      error: { code: -32602, message: '"params" must be an object' }
    })
  })
})

describe('a poll over the protocol', () => {
  it('returns a normalized page and hands since and a numeric limit to the trigger', async () => {
    const server = await serve()
    expect(
      await server.call('trigger/poll', {
        trigger: 'newTicket',
        since: '2026-08-01T00:00:00.000Z',
        limit: 25
      })
    ).toEqual({
      items: [
        {
          since: '2026-08-01T00:00:00.000Z',
          limit: 25,
          externalId: '1',
          title: 'Ticket 1',
          url: '',
          description: '',
          status: 'open',
          labels: [],
          updatedAt: NOW
        }
      ],
      hasMore: false
    })
  })

  it('refuses a poll it cannot run as asked', async () => {
    const server = await serve()
    expect(await server.fail('trigger/poll', { trigger: 'nope' })).toEqual({
      code: -32602,
      message: 'acme has no trigger "nope"'
    })
    expect(await server.fail('trigger/poll', { trigger: 'newTicket', limit: '25' })).toEqual({
      code: -32602,
      message: '"limit" must be a number'
    })
    expect(await server.fail('trigger/poll', { trigger: 'newTicket', cursor: 7 })).toMatchObject({
      code: -32602
    })
  })

  it('reports a failing poll as a connector error instead of dying', async () => {
    const server = await serve()
    expect(await server.fail('trigger/poll', { trigger: 'brokenTrigger' })).toEqual({
      code: -32000,
      message: 'upstream exploded',
      data: { kind: 'internal' }
    })
    expect(await server.call('trigger/poll', { trigger: 'newTicket' })).toMatchObject({
      hasMore: false
    })
  })

  it('reads its configuration only when a call needs it, and says what is missing', async () => {
    vi.stubEnv('API_TOKEN', '')
    const server = await greeted(connector, {})
    expect(await server.call('connector/manifest')).toMatchObject({ id: 'acme' })
    expect(await server.fail('trigger/poll', { trigger: 'newTicket' })).toMatchObject({
      code: -32000,
      message: expect.stringContaining('missing required configuration: apiToken (API_TOKEN)')
    })
  })
})

describe('an action over the protocol', () => {
  it('runs with resolved config and hands back everything it returned', async () => {
    const server = await serve()
    expect(await server.call('action/run', { action: 'closeTicket', args: { id: '7' } })).toEqual({
      closed: '7',
      token: 'tok'
    })
  })

  it('reports a missing argument as a validation error naming the field', async () => {
    const server = await serve()
    expect(
      await server.fail('action/run', { action: 'closeTicket', args: { reason: 'done' } })
    ).toEqual({
      code: -32000,
      message: 'Action closeTicket requires "id"',
      data: { kind: 'validation', field: 'id' }
    })
  })

  it('refuses an action it does not have, and arguments that are not an object', async () => {
    const server = await serve()
    expect(await server.fail('action/run', { action: 'reopenTicket', args: {} })).toEqual({
      code: -32602,
      message: 'acme has no action "reopenTicket"'
    })
    expect(await server.fail('action/run', { action: 'closeTicket', args: ['7'] })).toEqual({
      code: -32602,
      message: '"args" must be an object'
    })
  })

  it('returns lists, objects, null and whatever it declared, since outputs only document', async () => {
    const server = await greeted(
      defineConnector({
        id: 'shaped',
        name: 'Shaped',
        actions: [
          {
            type: 'listThings',
            label: 'List things',
            outputs: [
              { key: 'items', type: 'array', description: 'An array of things' },
              { key: 'owner', type: 'object', description: 'An object' },
              { key: 'note', type: 'string', description: 'Text, or null when there is none' },
              { key: 'count', type: 'number', description: 'How many' }
            ],
            run: () => ({
              items: [{ id: 1 }, { id: 2 }],
              owner: { name: 'Ada' },
              note: null,
              count: 'two'
            })
          }
        ]
      })
    )
    expect(await server.call('action/run', { action: 'listThings', args: {} })).toEqual({
      items: [{ id: 1 }, { id: 2 }],
      owner: { name: 'Ada' },
      note: null,
      count: 'two'
    })
  })
})

describe('the manifest a connector serves', () => {
  it('is the one it packs, naming the protocol it speaks', async () => {
    const server = await serve()
    const manifest = await server.call('connector/manifest')
    expect(manifest).toEqual(connectorManifest(connector))
    expect(manifest.protocol).toBe(1)
  })

  it('says whether an action is safe to retry and what each input is for', () => {
    const [close] = connectorManifest(connector).actions
    expect(close.idempotent).toBe(true)
    expect(close.inputs[1]).toEqual({
      key: 'reason',
      label: 'Reason',
      type: 'string',
      required: false,
      description: 'Why it was closed'
    })
    const risky = defineConnector({
      id: 'risky',
      name: 'Risky',
      actions: [{ type: 'create', label: 'Create', run: () => ({}) }]
    })
    expect(connectorManifest(risky).actions[0]).not.toHaveProperty('idempotent')
  })
})

describe('how a failure reads on the wire', () => {
  it('names the kind of failure, following what an error was wrapped around', () => {
    expect(protocolError(new ActionArgumentError('id', 'no id'))).toEqual({
      code: -32000,
      message: 'no id',
      data: { kind: 'validation', field: 'id' }
    })
    expect(protocolError(new SessionUnavailableError('Vorn is closed')).data).toEqual({
      kind: 'app-offline',
      retryable: false
    })
    const unauthorized = new UpstreamStatusError(401, 'Request failed with 401')
    expect(protocolError(unauthorized, true).data).toEqual({ kind: 'signed-out', retryable: false })
    expect(protocolError(unauthorized, false).data).toEqual({ kind: 'upstream', retryable: false })
    const wrapped = new Error('Action post: Request failed with 503', {
      cause: new UpstreamStatusError(503, 'Request failed with 503')
    })
    expect(protocolError(wrapped)).toEqual({
      code: -32000,
      message: 'Action post: Request failed with 503',
      data: { kind: 'upstream', retryable: true }
    })
    expect(protocolError('odd').data).toEqual({ kind: 'internal' })
  })
})

describe('connectionSetup', () => {
  it('names the trigger and the environment the connector reads', () => {
    expect(connectionSetup(connector, 'newTicket')).toEqual({
      connectorId: 'acme',
      triggerType: 'newTicket',
      env: [
        { name: 'API_TOKEN', required: true, secret: true },
        { name: 'ORG_URL', required: false, secret: false, description: 'Base URL' }
      ]
    })
  })

  it('rejects a trigger the connector does not have', () => {
    expect(() => connectionSetup(connector, 'nope')).toThrow(/has no trigger "nope"/)
  })
})

describe('vorn-connector CLI', () => {
  const capture = (): { lines: string[]; write: (line: string) => void } => {
    const lines: string[] = []
    return { lines, write: (line) => lines.push(line) }
  }
  const load = async (): Promise<unknown> => ({ default: connector })

  it('prints the manifest as JSON', async () => {
    const out = capture()
    expect(await runCli(['manifest', 'pkg'], { load, write: out.write })).toBe(0)
    expect(JSON.parse(out.lines.join('\n'))).toMatchObject({ id: 'acme', protocol: 1 })
  })

  it('prints each trigger and the environment it reads, for one trigger or all', async () => {
    const one = capture()
    await runCli(['setup', 'pkg', 'newTicket'], { load, write: one.write })
    expect(one.lines).toEqual([
      '# Acme — New ticket (newTicket)',
      'Environment: API_TOKEN (required), ORG_URL'
    ])

    const all = capture()
    await runCli(['setup', 'pkg'], { load, write: all.write })
    expect(all.lines.join('\n')).toContain('# Acme — Broken (brokenTrigger)')
  })

  it('checks a connector and fails only on errors', async () => {
    const warnings = capture()
    expect(await runCli(['check', 'pkg'], { load, write: warnings.write })).toBe(0)
    expect(warnings.lines.join('\n')).toContain('passed with')

    const broken = defineConnector({
      id: 'broken',
      name: 'Broken',
      description: 'Broken',
      triggers: [
        {
          type: 'stuck',
          label: 'Stuck',
          description: 'Ignores its cursor',
          poll: () => ({ items: [{ externalId: '1', title: 'One' }], nextCursor: 'same' })
        }
      ]
    })
    const errors = capture()
    expect(
      await runCli(['check', 'pkg', '--live'], {
        load: async () => ({ default: broken }),
        write: errors.write,
        env: {}
      })
    ).toBe(1)
    expect(errors.lines.join('\n')).toContain('redelivers-items')
    expect(errors.lines.join('\n')).toContain('1 error(s)')
  })

  it('polls against the supplied environment', async () => {
    const out = capture()
    const code = await runCli(['poll', 'pkg', 'newTicket', '--since', '2026-08-01T00:00:00.000Z'], {
      load,
      write: out.write,
      env: { API_TOKEN: 'tok' }
    })
    expect(code).toBe(0)
    expect(JSON.parse(out.lines.join('\n')).items[0].since).toBe('2026-08-01T00:00:00.000Z')
  })

  it('surfaces missing configuration rather than polling with none', async () => {
    const out = capture()
    await expect(
      runCli(['poll', 'pkg', 'newTicket'], { load, write: out.write, env: {} })
    ).rejects.toThrow(/missing required configuration/)
  })

  it('explains usage errors', async () => {
    const usage = capture()
    expect(await runCli([], { load, write: usage.write })).toBe(1)
    expect(usage.lines.join('\n')).toContain('vorn-connector <command>')

    const help = capture()
    expect(await runCli(['help'], { load, write: help.write })).toBe(0)

    const noModule = capture()
    expect(await runCli(['manifest'], { load, write: noModule.write })).toBe(1)
    expect(noModule.lines.join('\n')).toContain('Missing <module>')

    const noId = capture()
    expect(await runCli(['new'], { load, write: noId.write })).toBe(1)
    expect(noId.lines.join('\n')).toContain('Missing <id>')

    const noTrigger = capture()
    expect(await runCli(['poll', 'pkg'], { load, write: noTrigger.write })).toBe(1)
    expect(noTrigger.lines.join('\n')).toContain('Missing <trigger>')

    const badLimit = capture()
    expect(
      await runCli(['poll', 'pkg', 'newTicket', '--limit', 'lots'], {
        load,
        write: badLimit.write,
        env: { API_TOKEN: 'tok' }
      })
    ).toBe(1)
    expect(badLimit.lines.join('\n')).toContain('Invalid limit "lots"')

    const unknown = capture()
    expect(await runCli(['frobnicate', 'pkg'], { load, write: unknown.write })).toBe(1)
    expect(unknown.lines.join('\n')).toContain('Unknown command')
  })

  it('rejects a module that does not export a connector', async () => {
    const out = capture()
    await expect(
      runCli(['manifest', 'pkg'], { load: async () => ({}), write: out.write })
    ).rejects.toThrow(/does not export a connector/)
  })

  it('rejects a flag with no value', async () => {
    const out = capture()
    await expect(
      runCli(['poll', 'pkg', 'newTicket', '--since'], { load, write: out.write, env: {} })
    ).rejects.toThrow(/Missing value for --since/)
  })
})

describe('the CLI knowing it is the entry point', () => {
  it('matches its own file through a symlink, and never anything else', async () => {
    const { mkdtempSync, rmSync, symlinkSync } = await import('node:fs')
    const { tmpdir } = await import('node:os')
    const { join } = await import('node:path')
    const { fileURLToPath } = await import('node:url')
    const here = fileURLToPath(import.meta.url)
    const dir = mkdtempSync(join(tmpdir(), 'vorn-cli-entry-'))
    try {
      const link = join(dir, 'vorn-connector')
      symlinkSync(here, link)
      expect(isEntryPoint(import.meta.url, ['node', link])).toBe(true)
      expect(isEntryPoint(import.meta.url, ['node', '/nowhere/at/all'])).toBe(false)
      expect(isEntryPoint(import.meta.url, ['node'])).toBe(false)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
