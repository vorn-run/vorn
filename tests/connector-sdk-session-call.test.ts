import { afterEach, describe, expect, it, vi } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import {
  createConnectorServer,
  defineConnector,
  pollToolName,
  SESSION_CALL_HEADER,
  SESSION_CALL_META
} from '../packages/connector-sdk/src/index'
import {
  SESSION_CALL_HEADER as SHARED_HEADER,
  SESSION_CALL_META as SHARED_META
} from '../packages/shared/src/types'

const archive = 'https://example.com/archive'

const connector = defineConnector({
  id: 'pub',
  name: 'Pub',
  auth: {
    rung: 'browser',
    browser: {
      signInUrl: 'https://example.com/login',
      origins: ['https://example.com'],
      check: { url: 'https://example.com/me', identity: ['name'] }
    }
  },
  triggers: [
    {
      type: 'changed',
      label: 'Changed',
      dedupe: 'lastItem',
      fetch: async (ctx) => {
        await ctx.session!.fetch(archive)
        return []
      }
    }
  ],
  actions: [
    {
      type: 'peek',
      label: 'Peek',
      run: async (_args, ctx) => ({ status: (await ctx.session!.fetch(archive)).status })
    }
  ]
})

afterEach(() => {
  vi.unstubAllEnvs()
  vi.unstubAllGlobals()
})

/** The connector served in memory, with its window endpoint answered by a stub. */
async function served() {
  vi.stubEnv('VORN_BROWSER_HOST', 'http://127.0.0.1:4100/connections/c1/browser')
  vi.stubEnv('VORN_BROWSER_TOKEN', 't0k')
  const endpoint = vi.fn<typeof fetch>(
    async () => new Response(JSON.stringify({ status: 200, body: '{}' }))
  )
  vi.stubGlobal('fetch', endpoint)
  const server = createConnectorServer(connector, { config: {} })
  const client = new Client({ name: 'test', version: '1.0.0' })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)])
  return { client, endpoint }
}

describe('the key Vorn gives a tool call', () => {
  it('rides on each request an action makes through the window', async () => {
    const { client, endpoint } = await served()
    await client.callTool({ name: 'peek', arguments: {}, _meta: { [SESSION_CALL_META]: 'k1' } })
    expect(endpoint.mock.calls[0]?.[1]?.headers).toMatchObject({ [SESSION_CALL_HEADER]: 'k1' })
    await client.close()
  })

  it('rides on a poll too, so a poll that meets a sign-out is told apart', async () => {
    const { client, endpoint } = await served()
    await client.callTool({
      name: pollToolName('changed'),
      arguments: {},
      _meta: { [SESSION_CALL_META]: 'k2' }
    })
    expect(endpoint.mock.calls[0]?.[1]?.headers).toMatchObject({ [SESSION_CALL_HEADER]: 'k2' })
    await client.close()
  })

  it('is named the way the app reads it', () => {
    expect(SESSION_CALL_META).toBe(SHARED_META)
    expect(SESSION_CALL_HEADER).toBe(SHARED_HEADER)
  })
})
