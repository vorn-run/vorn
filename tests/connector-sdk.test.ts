import { describe, expect, it } from 'vitest'
import {
  createConnectorHarness,
  defineConnector,
  drainPoll,
  normalizeItem,
  resolveConfig,
  runAction,
  runPoll,
  envNameFor,
  connectorManifest
} from '../packages/connector-sdk/src/index'
import type { ConnectorItem, PollContext } from '../packages/connector-sdk/src/types'

const NOW = '2026-08-05T00:00:00.000Z'

const ticket = (id: string, updatedAt?: string): ConnectorItem => ({
  externalId: id,
  title: `Ticket ${id}`,
  url: `https://example.test/${id}`,
  ...(updatedAt && { updatedAt })
})

const simple = (poll: (context: PollContext) => unknown) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    triggers: [{ type: 'newTicket', label: 'New ticket', poll: poll as never }]
  })

describe('defineConnector', () => {
  it('fills in defaults for an otherwise minimal definition', () => {
    const connector = simple(() => ({ items: [] }))
    expect(connector.version).toBe('0.0.0')
    expect(connector.actions).toEqual([])
    expect(connector.config).toEqual([])
  })

  it('rejects definitions that could never be served', () => {
    expect(() => defineConnector({ id: '1bad', name: 'X' })).toThrow(/url-safe/)
    expect(() => defineConnector({ id: 'acme', name: '' })).toThrow(/missing a name/)
    expect(() => defineConnector({ id: 'acme', name: 'Acme' })).toThrow(
      /no triggers and no actions/
    )
  })

  it('rejects duplicate or malformed trigger and action keys', () => {
    const trigger = { type: 'a', label: 'A', poll: () => ({ items: [] }) }
    expect(() =>
      defineConnector({ id: 'acme', name: 'Acme', triggers: [trigger, { ...trigger }] })
    ).toThrow(/Duplicate trigger "a"/)
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        triggers: [{ type: 'has space', label: 'A', poll: () => ({ items: [] }) }]
      })
    ).toThrow(/url-safe/)
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        actions: [{ type: 'close', label: 'Close' } as never]
      })
    ).toThrow(/missing a run\(\)/)
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        triggers: [{ type: 'a', label: 'A' } as never]
      })
    ).toThrow(/missing a fetch\(\) or poll\(\)/)
  })

  it('rejects duplicate config keys', () => {
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        config: [
          { key: 'token', label: 'Token' },
          { key: 'token', label: 'Token again' }
        ],
        triggers: [{ type: 'a', label: 'A', poll: () => ({ items: [] }) }]
      })
    ).toThrow(/Duplicate config field "token"/)
  })
})

describe('resolveConfig', () => {
  const connector = defineConnector({
    id: 'acme',
    name: 'Acme',
    config: [
      { key: 'apiToken', label: 'API token', required: true, secret: true },
      { key: 'organizationUrl', label: 'Org URL', env: 'ACME_ORG_URL' },
      { key: 'pageSize', label: 'Page size', default: '50' }
    ],
    triggers: [{ type: 'a', label: 'A', poll: () => ({ items: [] }) }]
  })

  it('derives env names and applies defaults', () => {
    expect(envNameFor('apiToken')).toBe('API_TOKEN')
    expect(envNameFor('organizationUrl', 'ACME_ORG_URL')).toBe('ACME_ORG_URL')

    expect(resolveConfig(connector, { API_TOKEN: 't', ACME_ORG_URL: 'https://x' })).toEqual({
      apiToken: 't',
      organizationUrl: 'https://x',
      pageSize: '50'
    })
  })

  it('names every missing required value instead of failing at the first API call', () => {
    expect(() => resolveConfig(connector, {})).toThrow(/apiToken \(API_TOKEN\)/)
    expect(() => resolveConfig(connector, { API_TOKEN: '' })).toThrow(/missing required/)
  })
})

describe('normalizeItem', () => {
  it('fills every field Vorn reads by name', () => {
    expect(normalizeItem({ externalId: 7, title: 'Seven' }, NOW)).toEqual({
      externalId: '7',
      title: 'Seven',
      url: '',
      description: '',
      status: 'open',
      labels: [],
      updatedAt: NOW
    })
  })

  it('keeps extra data but never lets it shadow a reserved field', () => {
    const normalized = normalizeItem(
      {
        externalId: '1',
        title: 'Real title',
        assignee: 'dev',
        data: { title: 'spoofed', severity: 'high' }
      },
      NOW
    )
    expect(normalized.title).toBe('Real title')
    expect(normalized.severity).toBe('high')
    expect(normalized.assignee).toBe('dev')
  })

  it('normalizes Date timestamps and rejects unusable ones', () => {
    expect(
      normalizeItem({ externalId: '1', title: 't', updatedAt: new Date(NOW) }, NOW).updatedAt
    ).toBe(NOW)
    expect(() => normalizeItem({ externalId: '1', title: 't', updatedAt: 'soon' }, NOW)).toThrow(
      /Invalid updatedAt/
    )
  })

  it('drops prototype-polluting keys from extra data', () => {
    const normalized = normalizeItem(
      {
        externalId: '1',
        title: 't',
        data: JSON.parse('{"__proto__":{"polluted":true},"constructor":1,"safe":"yes"}')
      },
      NOW
    )
    expect(normalized.safe).toBe('yes')
    expect(normalized.constructor).toBe(Object)
    expect(({} as Record<string, unknown>).polluted).toBeUndefined()
  })

  it('rejects items that would be undeliverable', () => {
    expect(() => normalizeItem({ externalId: '  ', title: 't' }, NOW)).toThrow(/missing externalId/)
    expect(() => normalizeItem({ externalId: '1', title: ' ' }, NOW)).toThrow(/missing title/)
  })
})

describe('runPoll', () => {
  it('normalizes a page and defaults timestamps to poll time', async () => {
    const connector = simple(() => ({ items: [ticket('1')] }))
    const page = await runPoll(connector, 'newTicket', { now: () => NOW })
    expect(page).toEqual({
      items: [
        {
          externalId: '1',
          title: 'Ticket 1',
          url: 'https://example.test/1',
          description: '',
          status: 'open',
          labels: [],
          updatedAt: NOW
        }
      ],
      hasMore: false
    })
  })

  it('passes config, since, cursor and limit through to the author', async () => {
    let seen: PollContext | undefined
    const connector = simple((context) => {
      seen = context
      return { items: [] }
    })
    await runPoll(connector, 'newTicket', {
      config: { apiToken: 't' },
      since: NOW,
      cursor: 'c1',
      limit: 5,
      now: () => NOW
    })
    expect(seen).toMatchObject({ config: { apiToken: 't' }, since: NOW, cursor: 'c1', limit: 5 })
    expect(seen?.now()).toBe(NOW)
  })

  it('rejects a trigger that claims more pages without a cursor', async () => {
    const connector = simple(() => ({ items: [], hasMore: true }))
    await expect(runPoll(connector, 'newTicket', {})).rejects.toThrow(/without a nextCursor/)
  })

  it('rejects a malformed poll result or an unknown trigger', async () => {
    await expect(
      runPoll(
        simple(() => ({})),
        'newTicket',
        {}
      )
    ).rejects.toThrow(/items array/)
    await expect(
      runPoll(
        simple(() => ({ items: [] })),
        'nope',
        {}
      )
    ).rejects.toThrow(/has no trigger "nope"/)
  })

  it('rejects duplicate ids inside one page', async () => {
    const connector = simple(() => ({ items: [ticket('1'), ticket('1')] }))
    await expect(runPoll(connector, 'newTicket', { now: () => NOW })).rejects.toThrow(
      /Duplicate externalId "1"/
    )
  })
})

describe('drainPoll', () => {
  it('follows advancing cursors to the end of the backlog', async () => {
    const pages: Record<
      string,
      { items: ConnectorItem[]; nextCursor?: string; hasMore?: boolean }
    > = {
      start: { items: [ticket('1')], nextCursor: 'p2', hasMore: true },
      p2: { items: [ticket('2')] }
    }
    const connector = simple((context) => pages[context.cursor ?? 'start'])

    const items = await drainPoll(connector, 'newTicket', { now: () => NOW })
    expect(items.map((item) => item.externalId)).toEqual(['1', '2'])
  })

  it('stops a trigger whose cursor never moves', async () => {
    const connector = simple(() => ({ items: [], nextCursor: 'same', hasMore: true }))
    await expect(
      drainPoll(connector, 'newTicket', { cursor: 'same', now: () => NOW })
    ).rejects.toThrow(/did not advance its cursor/)
  })

  it('forwards an empty-string cursor instead of treating it as absent', async () => {
    const seen: (string | undefined)[] = []
    const connector = simple((context) => {
      seen.push(context.cursor)
      return { items: [ticket('1')] }
    })

    await drainPoll(connector, 'newTicket', { cursor: '', now: () => NOW })
    expect(seen).toEqual([''])
  })

  it('stops a trigger that pages forever', async () => {
    let page = 0
    const connector = simple(() => ({ items: [], nextCursor: `p${++page}`, hasMore: true }))
    await expect(drainPoll(connector, 'newTicket', { now: () => NOW })).rejects.toThrow(
      /exceeded 1000 pages/
    )
  })
})

describe('runAction', () => {
  const connector = defineConnector({
    id: 'acme',
    name: 'Acme',
    actions: [
      {
        type: 'closeTicket',
        label: 'Close ticket',
        inputs: [
          { key: 'id', label: 'Id', required: true },
          { key: 'number', label: 'Number', type: 'number' },
          { key: 'notify', label: 'Notify', type: 'boolean' }
        ],
        run: (args) => ({ received: args })
      },
      { type: 'ping', label: 'Ping', run: () => undefined }
    ]
  })

  it('coerces templated string arguments back to their declared types', async () => {
    expect(
      await runAction(connector, 'closeTicket', { id: '7', number: '42', notify: 'true' })
    ).toEqual({ received: { id: '7', number: 42, notify: true } })
  })

  it('drops blank optional arguments rather than passing empty strings upstream', async () => {
    expect(await runAction(connector, 'closeTicket', { id: '7', number: '' })).toEqual({
      received: { id: '7' }
    })
  })

  it('reports missing required and uncoercible arguments by name', async () => {
    await expect(runAction(connector, 'closeTicket', {})).rejects.toThrow(/requires "id"/)
    await expect(runAction(connector, 'closeTicket', { id: '1', number: 'x' })).rejects.toThrow(
      /argument "number": Expected a number/
    )
    await expect(runAction(connector, 'closeTicket', { id: '1', notify: 'treu' })).rejects.toThrow(
      /argument "notify": Expected a boolean/
    )
  })

  it('returns an empty object for an action with no output, and rejects unknown actions', async () => {
    expect(await runAction(connector, 'ping', {})).toEqual({})
    await expect(runAction(connector, 'nope', {})).rejects.toThrow(/has no action "nope"/)
  })
})

describe('createConnectorHarness', () => {
  it('exposes poll, drain, execute and manifest with shared defaults', async () => {
    const connector = defineConnector({
      id: 'acme',
      name: 'Acme',
      config: [{ key: 'apiToken', label: 'Token' }],
      triggers: [
        {
          type: 'newTicket',
          label: 'New ticket',
          poll: (context) => ({ items: [ticket(String(context.config.apiToken))] })
        }
      ],
      actions: [{ type: 'ping', label: 'Ping', run: (_args, ctx) => ({ at: ctx.now() }) }]
    })
    const harness = createConnectorHarness(connector, {
      config: { apiToken: 'tok' },
      now: () => NOW
    })

    expect((await harness.poll('newTicket')).items[0].externalId).toBe('tok')
    expect((await harness.drain('newTicket')).map((item) => item.title)).toEqual(['Ticket tok'])
    expect(await harness.execute('ping')).toEqual({ at: NOW })
    expect(harness.manifest().triggers[0].setup.filters.pollTool).toBe('poll_newTicket')
  })

  it('detects a trigger that ignores its lower bound and redelivers forever', async () => {
    const connector = simple(() => ({ items: [ticket('1', '2026-08-01T00:00:00.000Z')] }))
    const harness = createConnectorHarness(connector, { now: () => NOW })
    expect(await harness.pollTwice('newTicket')).toEqual([])
  })

  it('reports genuinely new items on the second poll', async () => {
    const later = '2026-08-02T00:00:00.000Z'
    let call = 0
    const connector = simple(() => ({
      items: call++ === 0 ? [ticket('1', '2026-08-01T00:00:00.000Z')] : [ticket('2', later)]
    }))
    const harness = createConnectorHarness(connector, { now: () => NOW })
    expect((await harness.pollTwice('newTicket')).map((item) => item.externalId)).toEqual(['2'])
  })
})

describe('connector icons', () => {
  const withIcon = (icon: unknown) =>
    defineConnector({
      id: 'acme',
      name: 'Acme',
      icon: icon as never,
      triggers: [{ type: 'newTicket', label: 'New ticket', poll: (() => ({ items: [] })) as never }]
    })

  it('keeps a valid icon and serves it in the manifest', () => {
    const connector = withIcon({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z'] })
    expect(connectorManifest(connector).icon).toEqual({
      viewBox: '0 0 16 16',
      paths: ['M1 1h4v4z']
    })
  })

  it('omits the icon from the manifest when the connector has none', () => {
    expect(connectorManifest(simple(() => ({ items: [] }))).icon).toBeUndefined()
  })

  it('accepts the full range of path commands and number notation', () => {
    expect(() =>
      withIcon({ paths: ['M1.5,-2 L3 4 H5 V6 C1 2 3 4 5 6 S1 2 3 4 Q1 2 3 4 A1 1 0 0 1 2e1 3 Z'] })
    ).not.toThrow()
  })

  it('rejects markup in path data, so an app can draw it without escaping', () => {
    expect(() => withIcon({ paths: ['"/><script>alert(1)</script>'] })).toThrow(/not SVG path data/)
  })

  it('rejects a url() reference that would pull in something external', () => {
    expect(() => withIcon({ paths: ['url(http://evil.test/x)'] })).toThrow(/not SVG path data/)
  })

  it('rejects an icon with no paths, which would render as nothing', () => {
    expect(() => withIcon({ paths: [] })).toThrow(/icon with no paths/)
    expect(() => withIcon({})).toThrow(/icon with no paths/)
  })

  it('rejects a viewBox that is not four numbers', () => {
    expect(() => withIcon({ viewBox: '0 0 24', paths: ['M1 1h4v4z'] })).toThrow(/four numbers/)
  })

  it('accepts a viewBox with negative and fractional bounds', () => {
    expect(() => withIcon({ viewBox: '-1.5 -1.5 27 27', paths: ['M1 1h4v4z'] })).not.toThrow()
  })
})
