import { describe, expect, it, vi } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import type { SourceConnection } from '../packages/shared/src/types'
import { createConnectorServer, defineConnector } from '../packages/connector-sdk/src/index'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const getOrStartClient = vi.fn()
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: (conn: SourceConnection) => getOrStartClient(conn)
}))

const NOW = '2026-08-05T00:00:00.000Z'

/**
 * The exact filters `connectionSetup()` tells a user to paste. Hard-coded here
 * rather than imported so a change to either side of the contract fails this
 * test instead of silently agreeing with itself.
 */
const SETUP_FILTERS = {
  pollTool: 'poll_newOrder',
  itemsPath: 'items',
  idField: 'externalId',
  timestampField: 'updatedAt',
  titleField: 'title',
  urlField: 'url'
}

const orders = [
  { id: 'o-1', reference: 'A', updatedAt: '2026-08-04T10:00:00.000Z' },
  { id: 'o-2', reference: 'B', updatedAt: '2026-08-04T11:00:00.000Z' }
]

const connector = defineConnector({
  id: 'orders-db',
  name: 'Orders database',
  triggers: [
    {
      type: 'newOrder',
      label: 'New order',
      poll: ({ since }) => ({
        items: orders
          .filter((order) => !since || order.updatedAt > since)
          .map((order) => ({
            externalId: order.id,
            title: `Order ${order.reference}`,
            url: `https://erp.test/${order.id}`,
            status: 'pending',
            updatedAt: order.updatedAt,
            data: { reference: order.reference }
          }))
      })
    }
  ],
  actions: [
    {
      type: 'shipOrder',
      label: 'Ship order',
      inputs: [{ key: 'id', label: 'Order id', required: true }],
      outputs: [{ key: 'shipped', description: 'Shipped order id' }],
      run: ({ id }) => ({ shipped: id, trackingNumber: 'TRACK-1' })
    }
  ]
})

async function connectSdkServer(): Promise<Client> {
  const server = createConnectorServer(connector, { config: {}, now: () => NOW })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  const client = new Client({ name: 'vorn', version: '1.0.0' })
  await Promise.all([client.connect(clientTransport), server.connect(serverTransport)])
  return client
}

async function connection(): Promise<SourceConnection> {
  const client = await connectSdkServer()
  getOrStartClient.mockResolvedValue(client)
  const { discoverTools } = await import('../packages/server/src/connectors/mcp')
  const base: SourceConnection = {
    id: 'conn-sdk',
    connectorId: 'mcp',
    name: 'Orders database',
    filters: { ...SETUP_FILTERS },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: NOW
  }
  return { ...base, filters: { ...base.filters, discoveredTools: await discoverTools(base) } }
}

describe('an SDK connector behind Vorn’s MCP connector', () => {
  it('polls into trigger events using only the generated setup filters', async () => {
    const conn = await connection()
    const { pollMcpConnection } = await import('../packages/server/src/connectors/mcp')

    const result = await pollMcpConnection(conn)

    expect(result.events.map((event) => event.id)).toEqual(['o-1', 'o-2'])
    expect(result.nextCursor).toBe('2026-08-04T11:00:00.000Z')
    expect(result.events[0].data).toMatchObject({
      externalId: 'o-1',
      title: 'Order A',
      url: 'https://erp.test/o-1',
      status: 'pending',
      reference: 'A'
    })
  })

  it('delivers nothing new once its cursor has caught up', async () => {
    const conn = await connection()
    const { pollMcpConnection } = await import('../packages/server/src/connectors/mcp')

    const first = await pollMcpConnection(conn)
    const second = await pollMcpConnection(conn, first.nextCursor)

    expect(second.events).toEqual([])
    expect(second.nextCursor).toBe(first.nextCursor)
  })

  it('exposes SDK actions as invocable MCP tools with typed output', async () => {
    const conn = await connection()
    const { invokeMcpTool } = await import('../packages/server/src/connectors/mcp')

    const result = await invokeMcpTool(conn, 'shipOrder', { id: 'o-1' })

    expect(result.success).toBe(true)
    expect(result.output).toMatchObject({ shipped: 'o-1', trackingNumber: 'TRACK-1' })
  })

  it('reports a connector-side failure as a failed action rather than a crash', async () => {
    const conn = await connection()
    const { invokeMcpTool } = await import('../packages/server/src/connectors/mcp')

    const result = await invokeMcpTool(conn, 'shipOrder', {})

    expect(result.success).toBe(false)
    expect(result.error).toContain('id')
  })

  it('renders SDK action inputs as connector action fields for the editor', async () => {
    const conn = await connection()
    const { mcpConnectionActions } = await import('../packages/server/src/connectors/mcp')

    const ship = mcpConnectionActions(conn).find((action) => action.type === 'shipOrder')

    expect(ship?.configFields.map((field) => field.key)).toEqual(['id'])
    expect(ship?.configFields[0].required).toBe(true)
    // Declared outputs reach the editor's variable autocomplete, while
    // undeclared ones (trackingNumber) still pass through at runtime.
    expect(Object.keys((ship?.outputSchema?.properties ?? {}) as Record<string, unknown>)).toEqual([
      'shipped'
    ])
  })
})
