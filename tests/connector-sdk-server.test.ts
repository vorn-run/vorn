import { describe, expect, it } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import {
  createConnectorServer,
  defineConnector,
  connectionSetup,
  connectorManifest,
  pollToolName,
  MANIFEST_TOOL
} from '../packages/connector-sdk/src/index'
import { runCli } from '../packages/connector-sdk/src/cli'
import type { Connector } from '../packages/connector-sdk/src/types'

const NOW = '2026-08-05T00:00:00.000Z'

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
      inputs: [
        { key: 'id', label: 'Id', required: true },
        { key: 'reason', label: 'Reason' }
      ],
      run: (args, context) => ({ closed: args.id, token: context.config.apiToken })
    }
  ]
})

type TextBlock = { type: string; text: string }
type ToolCallResult = { content: TextBlock[]; isError?: boolean }

async function connect(): Promise<Client> {
  const server = createConnectorServer(connector, {
    config: { apiToken: 'tok' },
    now: () => NOW
  })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  const client = new Client({ name: 'test', version: '1.0.0' })
  await Promise.all([client.connect(clientTransport), server.connect(serverTransport)])
  return client
}

function payload(result: unknown): unknown {
  return JSON.parse((result as ToolCallResult).content[0].text)
}

describe('connector MCP server', () => {
  it('serves one poll tool per trigger, one tool per action, and a manifest', async () => {
    const client = await connect()
    const names = (await client.listTools()).tools.map((tool) => tool.name).sort()
    expect(names).toEqual(['closeTicket', 'poll_brokenTrigger', 'poll_newTicket', MANIFEST_TOOL])
    await client.close()
  })

  it('returns a normalized page and forwards since/limit to the trigger', async () => {
    const client = await connect()
    const result = await client.callTool({
      name: pollToolName('newTicket'),
      arguments: { since: '2026-08-01T00:00:00.000Z', limit: '25' }
    })

    expect(payload(result)).toEqual({
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
    await client.close()
  })

  it('reports a failing poll as a tool error instead of killing the server', async () => {
    const client = await connect()
    const result = (await client.callTool({
      name: pollToolName('brokenTrigger'),
      arguments: {}
    })) as ToolCallResult
    expect(result.isError).toBe(true)
    expect(result.content[0].text).toContain('upstream exploded')

    const badLimit = (await client.callTool({
      name: pollToolName('newTicket'),
      arguments: { limit: 'many' }
    })) as ToolCallResult
    expect(badLimit.isError).toBe(true)
    expect(badLimit.content[0].text).toContain('Invalid limit')
    await client.close()
  })

  it('runs actions with resolved config and reports argument errors', async () => {
    const client = await connect()
    expect(payload(await client.callTool({ name: 'closeTicket', arguments: { id: '7' } }))).toEqual(
      {
        closed: '7',
        token: 'tok'
      }
    )

    const missing = (await client.callTool({
      name: 'closeTicket',
      arguments: { reason: 'done' }
    })) as ToolCallResult
    expect(missing.isError).toBe(true)
    await client.close()
  })

  it('advertises action inputs so the workflow editor can render a form', async () => {
    const client = await connect()
    const tool = (await client.listTools()).tools.find((entry) => entry.name === 'closeTicket')
    expect(Object.keys(tool?.inputSchema.properties ?? {})).toEqual(['id', 'reason'])
    expect(tool?.inputSchema.required).toEqual(['id'])
    await client.close()
  })

  it('serves the manifest a user needs to configure the connection', async () => {
    const client = await connect()
    expect(payload(await client.callTool({ name: MANIFEST_TOOL, arguments: {} }))).toEqual(
      connectorManifest(connector)
    )
    await client.close()
  })
})

describe('connectionSetup', () => {
  it('generates the exact filters a Vorn MCP connection needs', () => {
    expect(connectionSetup(connector, 'newTicket')).toEqual({
      connectorId: 'acme',
      triggerType: 'newTicket',
      filters: {
        pollTool: 'poll_newTicket',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt',
        titleField: 'title',
        urlField: 'url'
      },
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
    expect(JSON.parse(out.lines.join('\n')).id).toBe('acme')
  })

  it('prints setup for one trigger or for all of them', async () => {
    const one = capture()
    await runCli(['setup', 'pkg', 'newTicket'], { load, write: one.write })
    expect(one.lines.join('\n')).toContain('"pollTool": "poll_newTicket"')
    expect(one.lines.join('\n')).toContain('API_TOKEN (required)')

    const all = capture()
    await runCli(['setup', 'pkg'], { load, write: all.write })
    expect(all.lines.join('\n')).toContain('poll_brokenTrigger')
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

    const noTrigger = capture()
    expect(await runCli(['poll', 'pkg'], { load, write: noTrigger.write })).toBe(1)
    expect(noTrigger.lines.join('\n')).toContain('Missing <trigger>')

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
