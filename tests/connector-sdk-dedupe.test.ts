import { describe, expect, it } from 'vitest'
import {
  checkConnector,
  defineConnector,
  drainPoll,
  formatFindings,
  runPoll,
  type ConnectorItem,
  type FetchContext
} from '../packages/connector-sdk/src'

const NOW = '2026-08-05T12:00:00.000Z'

function ticket(id: string, updatedAt: string): ConnectorItem {
  return { externalId: id, title: `Ticket ${id}`, updatedAt }
}

/** A connector whose fetch records what the SDK asked for. */
function timestampConnector(
  fetch: (context: FetchContext) => ConnectorItem[] | Promise<ConnectorItem[]>
) {
  return defineConnector({
    id: 'acme',
    name: 'Acme',
    triggers: [{ type: 'newTicket', label: 'New ticket', dedupe: 'timestamp', fetch }]
  })
}

function lastItemConnector(
  fetch: (context: FetchContext) => ConnectorItem[] | Promise<ConnectorItem[]>
) {
  return defineConnector({
    id: 'acme',
    name: 'Acme',
    triggers: [{ type: 'newPost', label: 'New post', dedupe: 'lastItem', fetch }]
  })
}

describe('declarative triggers', () => {
  it('rejects a trigger that mixes or omits the two polling styles', () => {
    const base = { type: 'a', label: 'A' }
    expect(() =>
      defineConnector({
        id: 'x',
        name: 'X',
        triggers: [{ ...base, dedupe: 'timestamp', fetch: () => [], poll: () => ({ items: [] }) }]
      })
    ).toThrow(/declares both fetch\(\) and poll\(\)/)

    expect(() =>
      defineConnector({ id: 'x', name: 'X', triggers: [{ ...base, fetch: () => [] }] })
    ).toThrow(/fetch\(\) and a dedupe strategy together/)

    expect(() =>
      defineConnector({ id: 'x', name: 'X', triggers: [{ ...base, dedupe: 'timestamp' }] })
    ).toThrow(/fetch\(\) and a dedupe strategy together/)
  })

  it('rejects a misspelled dedupe strategy instead of guessing one', () => {
    expect(() =>
      defineConnector({
        id: 'x',
        name: 'X',
        triggers: [
          {
            type: 'a',
            label: 'A',
            // A plain-JS author gets no type error here.
            dedupe: 'timeStamp' as 'timestamp',
            fetch: () => []
          }
        ]
      })
    ).toThrow(/unknown dedupe strategy "timeStamp"; expected timestamp or lastItem/)
  })

  it('hands the author a since derived from its own cursor', async () => {
    const seen: (string | undefined)[] = []
    const connector = timestampConnector((context) => {
      seen.push(context.since)
      return [ticket('1', '2026-08-05T10:00:00.000Z')]
    })

    const first = await runPoll(connector, 'newTicket', { since: '2026-08-01T00:00:00.000Z' })
    await runPoll(connector, 'newTicket', { cursor: first.nextCursor!, now: () => NOW })

    expect(seen).toEqual(['2026-08-01T00:00:00.000Z', '2026-08-05T10:00:00.000Z'])
  })
})

describe('timestamp dedupe', () => {
  it('never redelivers items that share the newest timestamp', async () => {
    const shared = '2026-08-05T10:00:00.000Z'
    const items = [ticket('1', shared), ticket('2', shared), ticket('3', shared)]
    const connector = timestampConnector(() => items)

    const first = await runPoll(connector, 'newTicket', { now: () => NOW })
    expect(first.items.map((item) => item.externalId)).toEqual(['1', '2', '3'])

    // The source still returns all three; none of them may come back.
    const second = await runPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      now: () => NOW
    })
    expect(second.items).toEqual([])
  })

  it('still delivers a late item stamped at the boundary instant', async () => {
    const shared = '2026-08-05T10:00:00.000Z'
    let items = [ticket('1', shared)]
    const connector = timestampConnector(() => items)

    const first = await runPoll(connector, 'newTicket', { now: () => NOW })
    // A second item appears with the exact same timestamp after the first poll.
    items = [ticket('1', shared), ticket('2', shared)]

    const second = await runPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      now: () => NOW
    })
    expect(second.items.map((item) => item.externalId)).toEqual(['2'])
  })

  it('emits oldest first and leaves the newest behind when truncating', async () => {
    const connector = timestampConnector(() => [
      ticket('3', '2026-08-05T12:00:00.000Z'),
      ticket('1', '2026-08-05T10:00:00.000Z'),
      ticket('2', '2026-08-05T11:00:00.000Z')
    ])

    const first = await runPoll(connector, 'newTicket', { limit: 2, now: () => NOW })
    expect(first.items.map((item) => item.externalId)).toEqual(['1', '2'])
    // A first poll must not drain the source's whole history.
    expect(first.hasMore).toBe(false)

    const second = await runPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      limit: 2,
      now: () => NOW
    })
    expect(second.items.map((item) => item.externalId)).toEqual(['3'])
  })

  it('drains a truncated backlog once a cursor exists', async () => {
    const all = Array.from({ length: 5 }, (_, index) =>
      ticket(String(index), `2026-08-05T1${index}:00:00.000Z`)
    )
    const connector = timestampConnector(() => all)

    const first = await runPoll(connector, 'newTicket', { limit: 2, now: () => NOW })
    expect(first.hasMore).toBe(false)

    const rest = await drainPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      limit: 2,
      now: () => NOW
    })
    expect(rest.map((item) => item.externalId)).toEqual(['2', '3', '4'])
  })

  it('holds its cursor when a poll finds nothing', async () => {
    const connector = timestampConnector(() => [])
    const cursor = '{"v":1,"s":"timestamp","t":"2026-08-05T10:00:00.000Z","ids":["1"]}'
    const page = await runPoll(connector, 'newTicket', { cursor, now: () => NOW })
    expect(page).toEqual({ items: [], nextCursor: cursor, hasMore: false })
  })

  it('falls back to poll time for items with no timestamp', async () => {
    const connector = timestampConnector(() => [{ externalId: '1', title: 'No timestamp' }])
    const first = await runPoll(connector, 'newTicket', { now: () => NOW })
    expect(first.items[0]!.updatedAt).toBe(NOW)
    expect(JSON.parse(first.nextCursor!).t).toBe(NOW)
  })

  it('does not redeliver a timestamp-less item as poll time moves on', async () => {
    const connector = timestampConnector(() => [{ externalId: '1', title: 'No timestamp' }])
    const first = await runPoll(connector, 'newTicket', { now: () => NOW })

    // Poll time has advanced, but the item is the same one — giving it the new
    // poll time would make it look newer than the cursor on every single poll.
    const later = '2026-08-05T13:00:00.000Z'
    const second = await runPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      now: () => later
    })
    expect(second.items).toEqual([])
  })

  it('still recognizes a timestamp-less item after the boundary has moved past it', async () => {
    let rows: ConnectorItem[] = [{ externalId: 'x', title: 'No timestamp' }]
    const connector = timestampConnector(() => rows)
    const first = await runPoll(connector, 'newTicket', { now: () => NOW })
    expect(first.items.map((item) => item.externalId)).toEqual(['x'])

    // A properly stamped item arrives and drags the boundary forward. The
    // timestamp-less one must not come back with it.
    rows = [...rows, ticket('2', '2026-08-05T14:00:00.000Z')]
    const second = await runPoll(connector, 'newTicket', {
      cursor: first.nextCursor!,
      now: () => '2026-08-05T13:00:00.000Z'
    })
    expect(second.items.map((item) => item.externalId)).toEqual(['2'])

    const third = await runPoll(connector, 'newTicket', {
      cursor: second.nextCursor!,
      now: () => '2026-08-05T15:00:00.000Z'
    })
    expect(third.items).toEqual([])
  })

  it('rejects a cursor belonging to another strategy', async () => {
    const connector = timestampConnector(() => [])
    await expect(
      runPoll(connector, 'newTicket', { cursor: '{"v":1,"s":"lastItem","id":"9"}', now: () => NOW })
    ).rejects.toThrow(/does not belong to the "timestamp" strategy/)
    await expect(
      runPoll(connector, 'newTicket', { cursor: 'not json', now: () => NOW })
    ).rejects.toThrow(/not valid SDK cursor JSON/)
  })

  it('rejects a cursor whose payload is malformed', async () => {
    const connector = timestampConnector(() => [])
    await expect(
      runPoll(connector, 'newTicket', {
        cursor: '{"v":1,"s":"timestamp","t":"2026-01-01T00:00:00.000Z","ids":5}',
        now: () => NOW
      })
    ).rejects.toThrow(/missing the fields the "timestamp" strategy needs/)

    await expect(
      runPoll(
        lastItemConnector(() => []),
        'newPost',
        {
          cursor: '{"v":1,"s":"lastItem"}',
          now: () => NOW
        }
      )
    ).rejects.toThrow(/missing the fields the "lastItem" strategy needs/)
  })

  it('rejects a fetch that does not return an array', async () => {
    const connector = timestampConnector(() => undefined as never)
    await expect(runPoll(connector, 'newTicket', { now: () => NOW })).rejects.toThrow(
      /fetch\(\) did not return an array/
    )
  })
})

describe('lastItem dedupe', () => {
  const post = (id: string): ConnectorItem => ({ externalId: id, title: `Post ${id}` })

  it('stops at the newest id it already delivered and emits chronologically', async () => {
    let feed = [post('3'), post('2'), post('1')]
    const connector = lastItemConnector(() => feed)

    const first = await runPoll(connector, 'newPost', { now: () => NOW })
    expect(first.items.map((item) => item.externalId)).toEqual(['1', '2', '3'])

    feed = [post('5'), post('4'), post('3'), post('2'), post('1')]
    const second = await runPoll(connector, 'newPost', {
      cursor: first.nextCursor!,
      now: () => NOW
    })
    expect(second.items.map((item) => item.externalId)).toEqual(['4', '5'])
  })

  it('tells the author which id it already has', async () => {
    const seen: (string | undefined)[] = []
    const connector = lastItemConnector((context) => {
      seen.push(context.lastItemId)
      return [post('2'), post('1')]
    })

    const first = await runPoll(connector, 'newPost', { now: () => NOW })
    await runPoll(connector, 'newPost', { cursor: first.nextCursor!, now: () => NOW })
    expect(seen).toEqual([undefined, '2'])
  })

  it('advances only over what it delivered when truncating', async () => {
    const connector = lastItemConnector(() => [post('4'), post('3'), post('2'), post('1')])
    const first = await runPoll(connector, 'newPost', { limit: 2, now: () => NOW })
    expect(first.items.map((item) => item.externalId)).toEqual(['1', '2'])

    const second = await runPoll(connector, 'newPost', {
      cursor: first.nextCursor!,
      limit: 2,
      now: () => NOW
    })
    expect(second.items.map((item) => item.externalId)).toEqual(['3', '4'])
  })

  it('delivers everything when it has fallen off the end of the feed', async () => {
    const connector = lastItemConnector(() => [post('9'), post('8')])
    const page = await runPoll(connector, 'newPost', {
      cursor: '{"v":1,"s":"lastItem","id":"1"}',
      now: () => NOW
    })
    expect(page.items.map((item) => item.externalId)).toEqual(['8', '9'])
  })

  it('holds its cursor when the feed has nothing new', async () => {
    const connector = lastItemConnector(() => [post('1')])
    const cursor = '{"v":1,"s":"lastItem","id":"1"}'
    const page = await runPoll(connector, 'newPost', { cursor, now: () => NOW })
    expect(page).toEqual({ items: [], nextCursor: cursor, hasMore: false })
  })
})

describe('checkConnector', () => {
  const clean = defineConnector({
    id: 'acme',
    name: 'Acme',
    description: 'Acme tickets',
    triggers: [
      {
        type: 'newTicket',
        label: 'New ticket',
        description: 'Tickets opened since the last poll',
        dedupe: 'timestamp',
        fetch: () => [],
        sample: [ticket('1', '2026-08-05T10:00:00.000Z')]
      }
    ],
    actions: [
      {
        type: 'closeTicket',
        label: 'Close ticket',
        description: 'Close a ticket',
        idempotent: true,
        inputs: [{ key: 'id', label: 'Id', description: 'Ticket id' }],
        run: () => ({})
      }
    ]
  })

  it('passes a well-formed connector using only its samples', async () => {
    expect(await checkConnector(clean)).toEqual([])
  })

  it('catches a connector that redelivers its own items', async () => {
    const broken = defineConnector({
      id: 'broken',
      name: 'Broken',
      description: 'Broken',
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          description: 'Never advances',
          // A hand-rolled poll that ignores the cursor: the classic bug.
          poll: () => ({ items: [ticket('1', '2026-08-05T10:00:00.000Z')], nextCursor: 'same' })
        }
      ]
    })

    const findings = await checkConnector(broken, { live: true, config: {}, now: () => NOW })
    expect(findings.map((item) => item.code)).toContain('redelivers-items')
    expect(findings.find((item) => item.code === 'redelivers-items')!.level).toBe('error')
  })

  it('reports a trigger that returns items with no cursor', async () => {
    const broken = defineConnector({
      id: 'broken',
      name: 'Broken',
      description: 'Broken',
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          description: 'No cursor',
          poll: () => ({ items: [ticket('1', '2026-08-05T10:00:00.000Z')] })
        }
      ]
    })
    const findings = await checkConnector(broken, { live: true, config: {}, now: () => NOW })
    expect(findings.map((item) => item.code)).toEqual(['no-cursor'])
  })

  it('reports a trigger whose first poll throws', async () => {
    const broken = defineConnector({
      id: 'broken',
      name: 'Broken',
      description: 'Broken',
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          description: 'Throws',
          poll: () => {
            throw new Error('boom')
          }
        }
      ]
    })
    const findings = await checkConnector(broken, { live: true, config: {}, now: () => NOW })
    expect(findings[0]!.code).toBe('poll-failed')
    expect(findings[0]!.message).toContain('boom')
  })

  it('warns when nothing could be verified', async () => {
    const bare = defineConnector({
      id: 'bare',
      name: 'Bare',
      triggers: [{ type: 'newTicket', label: 'New ticket', poll: () => ({ items: [] }) }],
      actions: [{ type: 'ping', label: 'Ping', run: () => ({}) }]
    })

    const codes = (await checkConnector(bare)).map((item) => item.code)
    expect(codes).toContain('missing-description')
    expect(codes).toContain('unverifiable')
    expect(codes).toContain('missing-idempotent')
    expect((await checkConnector(bare)).every((item) => item.level === 'warn')).toBe(true)
  })

  it('warns that samples cannot be replayed through a hand-written poll', async () => {
    const manual = defineConnector({
      id: 'manual',
      name: 'Manual',
      description: 'Manual',
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          description: 'Manual',
          poll: () => ({ items: [] }),
          sample: [ticket('1', '2026-08-05T10:00:00.000Z')]
        }
      ]
    })
    expect((await checkConnector(manual)).map((item) => item.code)).toEqual(['sample-unusable'])
  })

  it('warns when a live poll returns nothing to check', async () => {
    const empty = defineConnector({
      id: 'empty',
      name: 'Empty',
      description: 'Empty',
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          description: 'Empty',
          dedupe: 'timestamp',
          fetch: () => []
        }
      ]
    })
    expect(
      (await checkConnector(empty, { live: true, config: {} })).map((item) => item.code)
    ).toEqual(['no-items'])
  })

  it('renders findings for a terminal', () => {
    expect(
      formatFindings([{ level: 'error', code: 'x', target: 'trigger a', message: 'broke' }])
    ).toBe('error  trigger a: broke [x]')
    expect(formatFindings([])).toBe('')
  })
})
