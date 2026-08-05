import { describe, expect, it, vi } from 'vitest'
import { createConnectorHarness, connectionSetup } from '../packages/connector-sdk/src'
import {
  createKustoConnector,
  parseLookback,
  toIsoTimestamp,
  withParameters,
  SINCE_PARAM,
  LIMIT_PARAM
} from '../packages/connector-kusto/src/connector'
import {
  normalizeClusterUrl,
  primaryTable,
  rowToRecord
} from '../packages/connector-kusto/src/client'
import { createTokenProvider } from '../packages/connector-kusto/src/auth'

const NOW = '2026-08-05T12:00:00.000Z'

const CONFIG = {
  cluster: 'help',
  database: 'Samples',
  query: 'Alerts | where FiredAt >= vorn_since | take vorn_limit'
}

const credential = {
  getToken: async () => ({ token: 't', expiresOnTimestamp: Date.now() + 3_600_000 })
}

/** Build a fetch stand-in that returns one v1 result table. */
function respondWith(columns: string[], rows: unknown[][]) {
  const calls: Array<{ url: string; body: Record<string, unknown> }> = []
  const fetchImpl = vi.fn(async (url: string, init: { body: string }) => {
    calls.push({ url, body: JSON.parse(init.body) })
    return {
      ok: true,
      status: 200,
      text: async () =>
        JSON.stringify({
          Tables: [
            {
              TableName: 'Table_0',
              Columns: columns.map((name) => ({ ColumnName: name })),
              Rows: rows
            },
            { TableName: 'QueryStatus', Columns: [], Rows: [] }
          ]
        })
    }
  })
  return { fetchImpl, calls }
}

function harness(fetchImpl: unknown, config: Record<string, string | undefined> = CONFIG) {
  const connector = createKustoConnector({
    credential,
    fetchImpl: fetchImpl as never
  })
  return createConnectorHarness(connector, { config, now: () => NOW })
}

describe('kusto connector', () => {
  describe('cluster urls', () => {
    it('completes a bare cluster name to the public endpoint', () => {
      expect(normalizeClusterUrl('help')).toBe('https://help.kusto.windows.net')
    })

    it('leaves a fully qualified host or url alone', () => {
      expect(normalizeClusterUrl('adx.eastus.kusto.windows.net')).toBe(
        'https://adx.eastus.kusto.windows.net'
      )
      expect(normalizeClusterUrl('https://adx.example.com/')).toBe('https://adx.example.com')
    })

    it('rejects an empty cluster', () => {
      expect(() => normalizeClusterUrl('  ')).toThrow(/empty/)
    })
  })

  describe('query parameters', () => {
    it('declares the poll window ahead of the author query', () => {
      expect(withParameters('Alerts | take 1')).toBe(
        `declare query_parameters(${SINCE_PARAM}:datetime, ${LIMIT_PARAM}:long);\nAlerts | take 1`
      )
    })

    it('refuses a query that declares its own parameters', () => {
      expect(() => withParameters('declare query_parameters(x:string);\nAlerts')).toThrow(
        /must not declare its own query parameters/
      )
    })

    it('binds the window as parameters rather than interpolating it', async () => {
      const { fetchImpl, calls } = respondWith(['Id', 'Timestamp'], [])
      await harness(fetchImpl).poll('queryResult', { since: '2026-08-05T11:00:00.000Z' })

      expect(calls[0].url).toBe('https://help.kusto.windows.net/v1/rest/query')
      expect(calls[0].body.properties).toEqual({
        Parameters: { [SINCE_PARAM]: '2026-08-05T11:00:00.000Z', [LIMIT_PARAM]: '100' }
      })
      // The user's text must survive untouched apart from the declaration.
      expect(calls[0].body.csl).toContain(CONFIG.query)
    })

    it('bounds the first poll by the lookback instead of replaying everything', async () => {
      const { fetchImpl, calls } = respondWith(['Id'], [])
      await harness(fetchImpl, { ...CONFIG, lookback: '2h' }).poll('queryResult')

      const params = (calls[0].body.properties as { Parameters: Record<string, string> }).Parameters
      expect(params[SINCE_PARAM]).toBe('2026-08-05T10:00:00.000Z')
    })

    it('defaults the lookback to an hour and rejects a malformed one', () => {
      expect(parseLookback(undefined)).toBe(3_600_000)
      expect(parseLookback('7d')).toBe(604_800_000)
      expect(() => parseLookback('soon')).toThrow(/use a value like/)
    })
  })

  describe('row mapping', () => {
    it('maps columns onto items and exposes every projected field', async () => {
      const { fetchImpl } = respondWith(
        ['Id', 'Timestamp', 'Title', 'Severity'],
        [['a1', '2026-08-05 11:30:00.0000000', 'Disk full', 3]]
      )
      const page = await harness(fetchImpl).poll('queryResult')

      expect(page.items).toHaveLength(1)
      expect(page.items[0]).toMatchObject({
        externalId: 'a1',
        title: 'Disk full',
        updatedAt: '2026-08-05T11:30:00.000Z'
      })
      // `data` is flattened onto the item, so a workflow templates it as
      // {{trigger.item.Severity}}.
      expect(page.items[0].Severity).toBe(3)
    })

    it('normalizes a zone-less Kusto datetime to a lexically sortable ISO string', () => {
      expect(toIsoTimestamp('2026-08-05 11:30:00.0000000')).toBe('2026-08-05T11:30:00.000Z')
      expect(toIsoTimestamp('2026-08-05T11:30:00Z')).toBe('2026-08-05T11:30:00.000Z')
      expect(toIsoTimestamp(new Date(NOW))).toBe(NOW)
      expect(toIsoTimestamp(null)).toBeUndefined()
      expect(toIsoTimestamp('not a date')).toBeUndefined()
    })

    it('falls back to the id when the title column is absent', async () => {
      const { fetchImpl } = respondWith(['Id'], [['a1']])
      const page = await harness(fetchImpl).poll('queryResult')
      expect(page.items[0].title).toBe('a1')
    })

    it('names the missing column when the query does not project an id', async () => {
      const { fetchImpl } = respondWith(['Name', 'Timestamp'], [['x', NOW]])
      await expect(harness(fetchImpl).poll('queryResult')).rejects.toThrow(
        /no "Id" column \(got: Name, Timestamp\)/
      )
    })

    it('refuses a row whose id is null rather than dedupe them together', async () => {
      const { fetchImpl } = respondWith(['Id'], [[null]])
      await expect(harness(fetchImpl).poll('queryResult')).rejects.toThrow(/needs a stable id/)
    })

    it('honours custom column names', async () => {
      const { fetchImpl } = respondWith(
        ['AlertId', 'FiredAt', 'Summary', 'Link'],
        [['z9', NOW, 'Boom', 'https://example.com/z9']]
      )
      const page = await harness(fetchImpl, {
        ...CONFIG,
        idColumn: 'AlertId',
        timestampColumn: 'FiredAt',
        titleColumn: 'Summary',
        urlColumn: 'Link'
      }).poll('queryResult')

      expect(page.items[0]).toMatchObject({
        externalId: 'z9',
        title: 'Boom',
        url: 'https://example.com/z9'
      })
    })
  })

  describe('dedupe', () => {
    it('does not redeliver rows Vorn has already seen', async () => {
      const { fetchImpl } = respondWith(
        ['Id', 'Timestamp'],
        [
          ['a', '2026-08-05T11:00:00Z'],
          ['b', '2026-08-05T11:30:00Z']
        ]
      )
      // The query is deliberately ignoring `vorn_since` here, which is the
      // common authoring mistake; the SDK cursor must still absorb it.
      const redelivered = await harness(fetchImpl).pollTwice('queryResult')
      expect(redelivered).toEqual([])
    })
  })

  describe('missing configuration', () => {
    it('names the environment variable that is missing', async () => {
      const { fetchImpl } = respondWith(['Id'], [])
      await expect(
        harness(fetchImpl, { database: 'Samples', query: 'Alerts' }).poll('queryResult')
      ).rejects.toThrow('KUSTO_CLUSTER is required')
      await expect(
        harness(fetchImpl, { cluster: 'help', query: 'Alerts' }).poll('queryResult')
      ).rejects.toThrow('KUSTO_DATABASE is required')
    })
  })

  describe('errors', () => {
    it('surfaces the innermost Kusto error message', async () => {
      const fetchImpl = async () => ({
        ok: false,
        status: 400,
        text: async () =>
          JSON.stringify({
            error: {
              message: 'Request is invalid',
              innererror: { message: "Failed to resolve entity 'Alertz'" }
            }
          })
      })
      await expect(harness(fetchImpl).poll('queryResult')).rejects.toThrow(
        /Kusto query failed \(400\): Failed to resolve entity 'Alertz'/
      )
    })

    it('keeps an unparseable error body verbatim', async () => {
      const fetchImpl = async () => ({ ok: false, status: 503, text: async () => 'upstream down' })
      await expect(harness(fetchImpl).poll('queryResult')).rejects.toThrow(/503\): upstream down/)
    })

    it('reports a malformed success body instead of a parser stack trace', async () => {
      const fetchImpl = async () => ({ ok: true, status: 200, text: async () => 'not json' })
      await expect(harness(fetchImpl).poll('queryResult')).rejects.toThrow(
        /Could not read the Kusto response/
      )
    })

    it('rejects a response with no tables', () => {
      expect(() => primaryTable({})).toThrow(/no tables/)
    })
  })

  describe('runQuery action', () => {
    it('returns rows keyed by column name', async () => {
      const { fetchImpl, calls } = respondWith(['Name', 'Count'], [['a', 1]])
      const result = await harness(fetchImpl).execute('runQuery', {
        query: 'Events | summarize Count=count() by Name'
      })

      expect(result).toMatchObject({ rowCount: 1, columns: ['Name', 'Count'] })
      expect(result.rows).toEqual([{ Name: 'a', Count: 1 }])
      // An ad-hoc query gets no injected parameters to reference.
      expect(calls[0].body.properties).toBeUndefined()
    })

    it('overrides the database when the step supplies one', async () => {
      const { fetchImpl, calls } = respondWith(['Name'], [])
      await harness(fetchImpl).execute('runQuery', { query: 'Events', database: 'Other' })
      expect(calls[0].body.db).toBe('Other')
    })

    it('requires a query that is more than whitespace', async () => {
      const { fetchImpl } = respondWith(['Name'], [])
      await expect(harness(fetchImpl).execute('runQuery', { query: '   ' })).rejects.toThrow(
        /query is required/
      )
    })
  })

  describe('token provider', () => {
    it('reuses a token until it nears expiry', async () => {
      const getToken = vi.fn(async () => ({ token: 'a', expiresOnTimestamp: 3_600_000 }))
      const provider = createTokenProvider({ credential: { getToken }, now: () => 0 })

      expect(await provider('https://help.kusto.windows.net')).toBe('a')
      expect(await provider('https://help.kusto.windows.net')).toBe('a')
      expect(getToken).toHaveBeenCalledTimes(1)
      expect(getToken).toHaveBeenCalledWith('https://help.kusto.windows.net/.default')
    })

    it('re-acquires once the token is inside the refresh margin', async () => {
      const getToken = vi.fn(async () => ({ token: 'a', expiresOnTimestamp: 60_000 }))
      const provider = createTokenProvider({ credential: { getToken }, now: () => 0 })
      await provider('https://c')
      await provider('https://c')
      expect(getToken).toHaveBeenCalledTimes(2)
    })

    it('keeps tokens for different clusters apart', async () => {
      const getToken = vi.fn(async (scope: string) => ({
        token: scope,
        expiresOnTimestamp: 3_600_000
      }))
      const provider = createTokenProvider({ credential: { getToken }, now: () => 0 })
      expect(await provider('https://a')).toBe('https://a/.default')
      expect(await provider('https://b')).toBe('https://b/.default')
    })

    it('explains how to sign in when no credential is available', async () => {
      const provider = createTokenProvider({ credential: { getToken: async () => null } })
      await expect(provider('https://c')).rejects.toThrow(/az login/)
    })

    it('explains how to sign in when the credential chain throws', async () => {
      const provider = createTokenProvider({
        credential: {
          getToken: async () => {
            throw new Error('no accounts')
          }
        }
      })
      await expect(provider('https://c')).rejects.toThrow(/az login.*no accounts/s)
    })
  })

  describe('setup', () => {
    it('exposes the environment variables Vorn must prompt for', () => {
      const setup = connectionSetup(createKustoConnector({ credential }), 'queryResult')
      const names = setup.env.map((entry) => entry.name)
      expect(names).toContain('KUSTO_CLUSTER')
      expect(names).toContain('KUSTO_QUERY')
      expect(setup.filters.pollTool).toBe('poll_queryResult')
    })
  })

  describe('rowToRecord', () => {
    it('pads a row that is shorter than the column list', () => {
      expect(rowToRecord(['a', 'b'], [1])).toEqual({ a: 1, b: undefined })
    })
  })
})

describe('kusto connector icon', () => {
  it('ships a glyph so the connection is not just another MCP row', () => {
    const icon = createKustoConnector({ fetch: async () => new Response('{}') }).icon
    expect(icon?.paths.length).toBeGreaterThan(0)
    expect(icon?.viewBox).toBe('0 0 24 24')
  })
})
