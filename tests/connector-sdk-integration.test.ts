import { describe, expect, it } from 'vitest'
import {
  defineConnector,
  defineExtension,
  type TriggerPollResult
} from '../packages/connector-sdk/src/index'
import { greeted, type Greeted } from './helpers/connector-server'

const NOW = '2026-08-05T00:00:00.000Z'

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
      dedupe: 'timestamp',
      fetch: ({ since }) =>
        orders
          .filter((order) => !since || order.updatedAt >= since)
          .map((order) => ({
            externalId: order.id,
            title: `Order ${order.reference}`,
            url: `https://erp.test/${order.id}`,
            status: 'pending',
            updatedAt: order.updatedAt,
            data: { reference: order.reference }
          }))
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

const poll = (server: Greeted, cursor?: string) =>
  server.call<TriggerPollResult>('trigger/poll', {
    trigger: 'newOrder',
    ...(cursor !== undefined && { cursor })
  })

describe('an SDK connector as Vorn polls it', () => {
  it('delivers normalized items and a cursor of its own', async () => {
    const server = await greeted(connector, { config: {}, now: () => NOW })

    const page = await poll(server)

    expect(page.items.map((item) => item.externalId)).toEqual(['o-1', 'o-2'])
    // The cursor Vorn stores is the connector's own opaque one, handed straight back next time.
    expect(JSON.parse(page.nextCursor!)).toMatchObject({ s: 'timestamp' })
    expect(page.items[0]).toMatchObject({
      externalId: 'o-1',
      title: 'Order A',
      url: 'https://erp.test/o-1',
      status: 'pending',
      reference: 'A'
    })
  })

  it('delivers nothing once its cursor has caught up', async () => {
    const server = await greeted(connector, { config: {}, now: () => NOW })

    const first = await poll(server)
    const second = await poll(server, first.nextCursor)

    // Not even the items sharing the newest instant come back.
    expect(second.items).toEqual([])
    expect(second.nextCursor).toBe(first.nextCursor)
  })

  it('runs an action and hands back everything it returned', async () => {
    const server = await greeted(connector, { config: {}, now: () => NOW })
    expect(await server.call('action/run', { action: 'shipOrder', args: { id: 'o-1' } })).toEqual({
      shipped: 'o-1',
      trackingNumber: 'TRACK-1'
    })
  })

  it('reports a missing argument as a failed call naming it, rather than a crash', async () => {
    const server = await greeted(connector, { config: {}, now: () => NOW })
    expect(await server.fail('action/run', { action: 'shipOrder', args: {} })).toMatchObject({
      message: expect.stringContaining('"id"'),
      data: { kind: 'validation', field: 'id' }
    })
  })

  it('drives a declarative dedupe trigger without the author writing cursor code', async () => {
    const shared = '2026-08-04T12:00:00.000Z'
    let rows = [{ id: 'r-1', updatedAt: shared }]
    const declarative = defineConnector({
      id: 'orders-db',
      name: 'Orders database',
      triggers: [
        {
          type: 'newOrder',
          label: 'New order',
          dedupe: 'timestamp',
          // The author only answers "what is there now?" — no cursor handling.
          fetch: () =>
            rows.map((row) => ({
              externalId: row.id,
              title: `Order ${row.id}`,
              updatedAt: row.updatedAt
            }))
        }
      ]
    })
    const server = await greeted(declarative, { config: {}, now: () => NOW })

    const first = await poll(server)
    expect(first.items.map((item) => item.externalId)).toEqual(['r-1'])

    // A second row at the exact same instant: the case `updatedAt > cursor` silently loses.
    rows = [...rows, { id: 'r-2', updatedAt: shared }]
    const second = await poll(server, first.nextCursor)
    expect(second.items.map((item) => item.externalId)).toEqual(['r-2'])
  })
})

describe('a served link handler', () => {
  it('answers with an empty result when the handler returns nothing', async () => {
    const extension = defineExtension({
      id: 'links',
      name: 'Links',
      permissions: [],
      linkHandlers: [
        {
          id: 'pr',
          title: 'Pull request',
          pattern: 'github\\.com/.+/pull/\\d+',
          example: 'https://github.com/vorn-run/vorn/pull/1',
          run: () => {}
        }
      ]
    })
    const server = await greeted(extension, { config: {}, now: () => NOW })

    expect(
      await server.call('extension/handler', {
        handler: 'pr',
        sessionId: 's1',
        worktreePath: '/tmp/w',
        agent: 'claude',
        url: 'https://github.com/vorn-run/vorn/pull/1'
      })
    ).toEqual({})
  })
})
