import net from 'node:net'
import { describe, expect, it } from 'vitest'
import type { Transport } from '@modelcontextprotocol/sdk/shared/transport.js'
import type { JSONRPCMessage } from '@modelcontextprotocol/sdk/types.js'
import type { VorndStatus } from '../packages/shared/src/types'
import { relay, relayHeaders, vorndMcpUrl } from '../packages/mcp/src/relay'

function health(groups: Record<string, { mode: string }>): typeof fetch {
  return (async () => new Response(JSON.stringify({ ok: true, groups }))) as typeof fetch
}

const on = (nativeServer: boolean) => async (): Promise<VorndStatus> => ({
  state: 'on',
  port: 4321,
  nativeServer
})

describe('whether to relay to vornd', () => {
  it('relays only when vornd serves the mcp group natively', async () => {
    const native = health({ mcp: { mode: 'native' } })
    expect((await vorndMcpUrl({ vorndStatus: on(true), fetch: native }))?.href).toBe(
      'http://127.0.0.1:4321/mcp'
    )
    expect(await vorndMcpUrl({ vorndStatus: on(false), fetch: native })).toBeNull()
    expect(
      await vorndMcpUrl({ vorndStatus: async () => ({ state: 'off' }), fetch: native })
    ).toBeNull()
    expect(
      await vorndMcpUrl({ vorndStatus: on(true), fetch: health({ mcp: { mode: 'forward' } }) })
    ).toBeNull()
    expect(await vorndMcpUrl({ vorndStatus: on(true), fetch: health({}) })).toBeNull()
  })

  it('serves the tools itself when the server or vornd cannot be asked', async () => {
    const failing = async (): Promise<never> => {
      throw new Error('Method not found: server:vornd')
    }
    expect(
      await vorndMcpUrl({ vorndStatus: failing, fetch: health({ mcp: { mode: 'native' } }) })
    ).toBeNull()
    const unreachable = (async () => {
      throw new TypeError('fetch failed')
    }) as typeof fetch
    expect(await vorndMcpUrl({ vorndStatus: on(true), fetch: unreachable })).toBeNull()
  })
})

describe('relayHeaders', () => {
  it("carries the credential, the agent's directory and its session", () => {
    expect(relayHeaders('tok', '/home/me/my app', { VORN_SESSION_ID: 's-1' })).toEqual({
      Authorization: 'Bearer tok',
      'Vorn-Cwd': '%2Fhome%2Fme%2Fmy%20app',
      'Vorn-Session-Id': 's-1'
    })
    expect(relayHeaders('tok', '/', { VORN_SESSION_ID: '' })).not.toHaveProperty('Vorn-Session-Id')
  })
})

/** A transport the test plays the agent's side of. */
class Agent implements Transport {
  onmessage?: (message: JSONRPCMessage) => void
  onclose?: () => void
  received: JSONRPCMessage[] = []
  async start(): Promise<void> {}
  async send(message: JSONRPCMessage): Promise<void> {
    this.received.push(message)
  }
  async close(): Promise<void> {
    this.onclose?.()
  }
}

/** A port nothing listens on. */
async function closedPort(): Promise<number> {
  const server = net.createServer()
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = (server.address() as net.AddressInfo).port
  await new Promise<void>((resolve) => server.close(() => resolve()))
  return port
}

describe('relay', () => {
  it('answers a request vornd cannot be reached for, and ends with the agent', async () => {
    const agent = new Agent()
    const url = new URL(`http://127.0.0.1:${await closedPort()}/mcp`)
    const done = relay(agent, url, relayHeaders('tok', '/', {}))
    await new Promise((resolve) => setImmediate(resolve))
    agent.onmessage!({ jsonrpc: '2.0', method: 'notifications/initialized' })
    agent.onmessage!({ jsonrpc: '2.0', id: 9, method: 'ping' })
    await expect.poll(() => agent.received.length).toBe(1)
    const [answer] = agent.received as Array<{
      id: number
      error: { code: number; message: string }
    }>
    expect(answer.id).toBe(9)
    expect(answer.error.code).toBe(-32603)
    expect(answer.error.message).toMatch(/^vornd's MCP server did not answer: /)
    await agent.close()
    await done
  })
})
