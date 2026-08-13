import { describe, expect, it } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import {
  createConnectorServer,
  defineConnector,
  PREFLIGHT_TOOL
} from '../packages/connector-sdk/src/index'
import type { ConnectorDefinition } from '../packages/connector-sdk/src/types'

/**
 * The preflight tool exists for connectors whose credentials come from an
 * external login rather than a config field — there is no field to be missing,
 * so without this the first sign of trouble is a poll failing minutes later.
 */
async function connect(definition: ConnectorDefinition): Promise<Client> {
  const server = createConnectorServer(defineConnector(definition), { config: {} })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  const client = new Client({ name: 'test', version: '1.0.0' })
  await Promise.all([client.connect(clientTransport), server.connect(serverTransport)])
  return client
}

const base: ConnectorDefinition = {
  id: 'acme',
  name: 'Acme',
  version: '1.0.0',
  triggers: [{ type: 't', label: 'T', poll: () => ({ items: [] }) }]
}

async function toolNames(client: Client): Promise<string[]> {
  return (await client.listTools()).tools.map((t) => t.name)
}

function structured(result: unknown): Record<string, unknown> {
  return (result as { structuredContent: Record<string, unknown> }).structuredContent
}

describe('the preflight tool', () => {
  // Absence is the signal for "nothing to check". Registering it
  // unconditionally would make every connector claim to have been verified.
  it('is absent when the connector declares no preflight', async () => {
    const client = await connect(base)
    expect(await toolNames(client)).not.toContain(PREFLIGHT_TOOL)
  })

  it('is present when the connector declares one', async () => {
    const client = await connect({ ...base, preflight: () => ({ ok: true }) })
    expect(await toolNames(client)).toContain(PREFLIGHT_TOOL)
  })

  it('reports a passing check', async () => {
    const client = await connect({ ...base, preflight: () => ({ ok: true }) })
    expect(structured(await client.callTool({ name: PREFLIGHT_TOOL, arguments: {} }))).toEqual({
      ok: true
    })
  })

  it('carries the message back, because it says what to do about it', async () => {
    const client = await connect({
      ...base,
      preflight: () => ({ ok: false, message: 'Sign in by running `gh auth login`.' })
    })
    expect(structured(await client.callTool({ name: PREFLIGHT_TOOL, arguments: {} }))).toEqual({
      ok: false,
      message: 'Sign in by running `gh auth login`.'
    })
  })

  it('awaits an async preflight rather than reporting the promise', async () => {
    const client = await connect({
      ...base,
      preflight: async () => {
        await Promise.resolve()
        return { ok: false, message: 'still no' }
      }
    })
    expect(structured(await client.callTool({ name: PREFLIGHT_TOOL, arguments: {} }))).toEqual({
      ok: false,
      message: 'still no'
    })
  })

  // A throw means "broken", which must not reach the user as a passing check.
  // Reported as a failed preflight so the caller has one shape to read.
  it('turns a throw into a failed check carrying its message', async () => {
    const client = await connect({
      ...base,
      preflight: () => {
        throw new Error('gh not found on PATH')
      }
    })
    const result = await client.callTool({ name: PREFLIGHT_TOOL, arguments: {} })
    expect(structured(result)).toEqual({ ok: false, message: 'gh not found on PATH' })
    // Not an MCP error: the call succeeded in reporting a real answer.
    expect(result.isError).toBeFalsy()
  })

  it('survives a rejected promise the same way', async () => {
    const client = await connect({
      ...base,
      preflight: () => Promise.reject(new Error('signed out'))
    })
    expect(structured(await client.callTool({ name: PREFLIGHT_TOOL, arguments: {} }))).toEqual({
      ok: false,
      message: 'signed out'
    })
  })
})
