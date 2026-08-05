/**
 * Thin Azure Data Explorer query client.
 *
 * Uses the v1 REST endpoint, whose response is a plain `{ Tables: [...] }`
 * document, rather than v2's frame stream — the extra fidelity v2 offers is
 * all about progressive results, which a poll that waits for the whole answer
 * cannot use anyway.
 */

/** Accepts `help`, `help.kusto.windows.net`, or a full https URL. */
export function normalizeClusterUrl(raw: string): string {
  const trimmed = raw.trim().replace(/\/+$/, '')
  if (!trimmed) throw new Error('Kusto cluster is empty')
  if (/^http:\/\//i.test(trimmed)) {
    // Every request carries an Entra bearer token, so plaintext is refused
    // rather than silently upgraded — a misconfigured cluster should be a
    // setup error, not a token on the wire.
    throw new Error(`Kusto cluster must use https, got "${trimmed}"`)
  }
  if (/^https:\/\//i.test(trimmed)) return trimmed
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)) {
    throw new Error(`Kusto cluster must be a cluster name or an https URL, got "${trimmed}"`)
  }
  // A bare name is by far the most common way people refer to a cluster, and
  // the regional suffix is not guessable, so only the well-known public form
  // is completed for them.
  if (trimmed.includes('.')) return `https://${trimmed}`
  return `https://${trimmed}.kusto.windows.net`
}

export interface KustoTable {
  columns: string[]
  rows: unknown[][]
}

interface V1Response {
  Tables?: Array<{
    TableName?: string
    Columns?: Array<{ ColumnName?: string }>
    Rows?: unknown[][]
  }>
}

/**
 * Pull the query's own result out of a v1 response.
 *
 * A v1 query returns the result table first, followed by QueryStatus and
 * QueryProperties tables that describe the run. Taking `Tables[0]` is
 * therefore correct, but only by convention, so the shape is checked.
 */
export function primaryTable(body: unknown): KustoTable {
  const tables = (body as V1Response)?.Tables
  if (!Array.isArray(tables) || tables.length === 0) {
    throw new Error('Kusto returned no tables')
  }
  const table = tables[0]
  const columns = (table?.Columns ?? []).map(
    (column, index) => column?.ColumnName ?? `Column${index}`
  )
  const rows = Array.isArray(table?.Rows) ? table.Rows : []
  return { columns, rows }
}

/**
 * Turn a row array into a keyed object using the table's column names.
 *
 * Null-prototype, because the query author picks the column names: on a plain
 * object a column projected as `__proto__` would set the record's prototype
 * instead of becoming a property, so the value would vanish from the item and
 * the record would carry whatever the row supplied.
 */
export function rowToRecord(columns: string[], row: unknown[]): Record<string, unknown> {
  const record: Record<string, unknown> = Object.create(null) as Record<string, unknown>
  columns.forEach((column, index) => {
    record[column] = row[index]
  })
  return record
}

export type FetchLike = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string }
) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>

export interface KustoQueryOptions {
  clusterUrl: string
  database: string
  query: string
  /** Bound as KQL query parameters, never interpolated into the query text. */
  parameters?: Record<string, string>
  getToken(clusterUrl: string): Promise<string>
  fetchImpl?: FetchLike
}

export async function runKustoQuery(options: KustoQueryOptions): Promise<KustoTable> {
  const { clusterUrl, database, query, parameters, getToken } = options
  const doFetch = options.fetchImpl ?? (globalThis.fetch as unknown as FetchLike)
  const token = await getToken(clusterUrl)

  const response = await doFetch(`${clusterUrl}/v1/rest/query`, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${token}`,
      'content-type': 'application/json',
      accept: 'application/json'
    },
    body: JSON.stringify({
      db: database,
      csl: query,
      ...(parameters && Object.keys(parameters).length > 0
        ? { properties: { Parameters: parameters } }
        : {})
    })
  })

  const text = await response.text()
  if (!response.ok) {
    throw new Error(`Kusto query failed (${response.status}): ${kustoError(text)}`)
  }
  try {
    return primaryTable(JSON.parse(text))
  } catch (error) {
    throw new Error(
      `Could not read the Kusto response: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error }
    )
  }
}

/**
 * Kusto nests the useful part of an error several levels down and repeats a
 * generic message at the top, so surface the innermost text when it is there.
 */
function kustoError(body: string): string {
  try {
    const parsed = JSON.parse(body) as {
      error?: { message?: string; '@message'?: string; innererror?: { message?: string } }
    }
    const inner = parsed.error?.innererror?.message
    return inner ?? parsed.error?.['@message'] ?? parsed.error?.message ?? body
  } catch {
    return body
  }
}
