import { defineConnector, type ConnectorItem, type FetchContext } from '@vornrun/connector-sdk'
import { createTokenProvider, type TokenCredentialLike } from './auth'
import { normalizeClusterUrl, rowToRecord, runKustoQuery, type FetchLike } from './client'

/**
 * KQL parameters the connector always declares.
 *
 * A query is user-supplied, so the poll window is bound as a real query
 * parameter rather than pasted into the text — string interpolation here would
 * be a KQL injection with the connector's own credentials behind it.
 */
export const SINCE_PARAM = 'vorn_since'
export const LIMIT_PARAM = 'vorn_limit'

const PARAM_DECLARATION = `declare query_parameters(${SINCE_PARAM}:datetime, ${LIMIT_PARAM}:long);`

const DEFAULT_LOOKBACK = '1h'
const DEFAULT_LIMIT = 100

const DURATION_MS: Record<string, number> = {
  m: 60_000,
  h: 3_600_000,
  d: 86_400_000
}

/** Parse a lookback like `30m`, `2h`, or `7d` into milliseconds. */
export function parseLookback(raw: string | undefined): number {
  const value = (raw ?? DEFAULT_LOOKBACK).trim()
  const match = /^(\d+)\s*([mhd])$/i.exec(value)
  if (!match) {
    throw new Error(`Invalid lookback "${value}"; use a value like 30m, 2h or 7d`)
  }
  return Number(match[1]) * DURATION_MS[match[2].toLowerCase()]
}

/**
 * Prefix the author's query with the parameter declaration.
 *
 * KQL requires `declare query_parameters` to lead the query, so a query that
 * already declares its own would end up with two declaration statements.
 */
export function withParameters(query: string): string {
  const trimmed = query.trim()
  if (!trimmed) throw new Error('KUSTO_QUERY is empty')
  if (/^\s*declare\s+query_parameters/i.test(trimmed)) {
    throw new Error(
      `KUSTO_QUERY must not declare its own query parameters; ` +
        `${SINCE_PARAM} and ${LIMIT_PARAM} are declared for you`
    )
  }
  return `${PARAM_DECLARATION}\n${trimmed}`
}

function required(config: Record<string, string | undefined>, key: string, env: string): string {
  const value = config[key]?.trim()
  if (!value) throw new Error(`${env} is required`)
  return value
}

/**
 * Kusto renders datetimes with a variable number of fractional digits and no
 * trailing `Z`, which sorts incorrectly as a plain string. Vorn compares
 * `updatedAt` lexically, so every timestamp is normalized to ISO 8601 UTC.
 */
export function toIsoTimestamp(value: unknown): string | undefined {
  if (value === null || value === undefined || value === '') return undefined
  if (value instanceof Date) return Number.isNaN(value.getTime()) ? undefined : value.toISOString()
  const raw = String(value)
  // A bare Kusto datetime has no zone; it is always UTC.
  const candidate = /(Z|[+-]\d{2}:?\d{2})$/.test(raw) ? raw : `${raw}Z`
  const parsed = new Date(candidate)
  return Number.isNaN(parsed.getTime()) ? undefined : parsed.toISOString()
}

function text(value: unknown): string | undefined {
  if (value === null || value === undefined) return undefined
  const str = String(value)
  return str === '' ? undefined : str
}

export interface KustoConnectorOptions {
  /** Injected by tests; production uses `DefaultAzureCredential` and `fetch`. */
  credential?: TokenCredentialLike
  fetchImpl?: FetchLike
  now?: () => number
  /** Reported in the MCP handshake; supplied by the entry point. */
  version?: string
}

export function createKustoConnector(options: KustoConnectorOptions = {}) {
  const getToken = createTokenProvider({
    ...(options.credential && { credential: options.credential }),
    ...(options.now && { now: options.now })
  })

  async function fetchRows(context: FetchContext): Promise<ConnectorItem[]> {
    const { config } = context
    const clusterUrl = normalizeClusterUrl(required(config, 'cluster', 'KUSTO_CLUSTER'))
    const database = required(config, 'database', 'KUSTO_DATABASE')
    const query = withParameters(required(config, 'query', 'KUSTO_QUERY'))

    const idColumn = config.idColumn?.trim() || 'Id'
    const timestampColumn = config.timestampColumn?.trim() || 'Timestamp'
    const titleColumn = config.titleColumn?.trim() || 'Title'
    const urlColumn = config.urlColumn?.trim()

    // On the first poll there is no watermark, so the lookback bounds the
    // window. Without it, connecting a trigger would replay the entire table.
    const since =
      context.since ??
      new Date(Date.parse(context.now()) - parseLookback(config.lookback)).toISOString()
    const limit = context.limit ?? DEFAULT_LIMIT

    const table = await runKustoQuery({
      clusterUrl,
      database,
      query,
      parameters: { [SINCE_PARAM]: since, [LIMIT_PARAM]: String(limit) },
      getToken,
      ...(options.fetchImpl && { fetchImpl: options.fetchImpl })
    })

    if (!table.columns.includes(idColumn)) {
      throw new Error(
        `Query result has no "${idColumn}" column (got: ${table.columns.join(', ') || 'no columns'}). ` +
          `Set KUSTO_ID_COLUMN, or project the column in the query.`
      )
    }

    return table.rows.map((row) => {
      const record = rowToRecord(table.columns, row)
      const externalId = text(record[idColumn])
      if (externalId === undefined) {
        // Vorn dedupes on this, so a null id would collapse distinct rows into
        // one event or replay them forever depending on the strategy.
        throw new Error(`Row has an empty "${idColumn}"; every row needs a stable id`)
      }
      const updatedAt = toIsoTimestamp(record[timestampColumn])
      return {
        externalId,
        title: text(record[titleColumn]) ?? externalId,
        ...(updatedAt !== undefined && { updatedAt }),
        ...(urlColumn && { url: text(record[urlColumn]) ?? '' }),
        // Every projected column is exposed, so a workflow can template any
        // field the query selected without the connector knowing about it.
        data: record
      }
    })
  }

  return defineConnector({
    id: 'kusto',
    name: 'Azure Data Explorer',
    ...(options.version && { version: options.version }),
    description: 'Trigger workflows from the rows a KQL query returns.',
    config: [
      {
        key: 'cluster',
        env: 'KUSTO_CLUSTER',
        label: 'Cluster',
        required: true,
        description: 'Cluster name or URL, e.g. "help" or https://help.kusto.windows.net'
      },
      { key: 'database', env: 'KUSTO_DATABASE', label: 'Database', required: true },
      {
        key: 'query',
        env: 'KUSTO_QUERY',
        label: 'KQL query',
        required: true,
        description: `Query to poll. Reference ${SINCE_PARAM} and ${LIMIT_PARAM} to bound it.`
      },
      {
        key: 'idColumn',
        env: 'KUSTO_ID_COLUMN',
        label: 'Id column',
        default: 'Id',
        description: 'Column holding a stable per-row id. Vorn dedupes on it.'
      },
      {
        key: 'timestampColumn',
        env: 'KUSTO_TIMESTAMP_COLUMN',
        label: 'Timestamp column',
        default: 'Timestamp',
        description: 'Column the poll watermark advances from.'
      },
      { key: 'titleColumn', env: 'KUSTO_TITLE_COLUMN', label: 'Title column', default: 'Title' },
      { key: 'urlColumn', env: 'KUSTO_URL_COLUMN', label: 'Url column' },
      {
        key: 'lookback',
        env: 'KUSTO_LOOKBACK',
        label: 'First-poll lookback',
        default: DEFAULT_LOOKBACK,
        description: 'How far back the very first poll looks, e.g. 30m, 2h, 7d.'
      }
    ],
    triggers: [
      {
        type: 'queryResult',
        label: 'Query returns a row',
        description: 'Fires once per new row returned by the configured KQL query.',
        dedupe: 'timestamp',
        fetch: fetchRows
      }
    ],
    actions: [
      {
        type: 'runQuery',
        label: 'Run a KQL query',
        description: 'Run a read-only query and return its rows.',
        // Kusto queries cannot mutate data, so a retried step is always safe.
        idempotent: true,
        inputs: [
          { key: 'query', label: 'KQL query', required: true },
          { key: 'database', label: 'Database', description: 'Defaults to KUSTO_DATABASE' }
        ],
        outputs: [
          { key: 'rowCount', type: 'number', description: 'Number of rows returned' },
          { key: 'columns', description: 'Column names, in order' }
        ],
        async run(args, { config }) {
          const clusterUrl = normalizeClusterUrl(required(config, 'cluster', 'KUSTO_CLUSTER'))
          const query = String(args.query ?? '').trim()
          if (!query) throw new Error('query is required')
          const database =
            String(args.database ?? '').trim() || required(config, 'database', 'KUSTO_DATABASE')

          const table = await runKustoQuery({
            clusterUrl,
            database,
            query,
            getToken,
            ...(options.fetchImpl && { fetchImpl: options.fetchImpl })
          })
          return {
            rowCount: table.rows.length,
            columns: table.columns,
            rows: table.rows.map((row) => rowToRecord(table.columns, row))
          }
        }
      }
    ]
  })
}
