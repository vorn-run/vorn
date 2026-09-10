// The pure half of a connection's signed-in profile, kept apart from Electron so it can be tested.

/** The largest body a signed-in call hands back, so a runaway page cannot fill memory. */
export const MAX_SESSION_BODY = 4 * 1024 * 1024

export interface SessionRequest {
  url: string
  method: string
  headers?: Record<string, string>
  body?: string
}

export interface SessionAnswer {
  status: number
  headers: Record<string, string>
  body: string
}

/** The only request headers a call may set; the page supplies the rest itself, cookies included. */
const ALLOWED_HEADERS = new Set(['accept', 'content-type'])

export function allowedHeaders(headers: Record<string, string> = {}): Record<string, string> {
  return Object.fromEntries(
    Object.entries(headers).filter(([name]) => ALLOWED_HEADERS.has(name.toLowerCase()))
  )
}

/** The script a runner page executes, built from JSON so nothing a connector sends runs as code. */
export function fetchScript(request: SessionRequest): string {
  const init = {
    method: request.method.toUpperCase(),
    headers: allowedHeaders(request.headers),
    credentials: 'include',
    ...(request.body !== undefined && { body: request.body })
  }
  return `(async () => {
  const res = await fetch(${JSON.stringify(request.url)}, ${JSON.stringify(init)})
  const text = await res.text()
  return { status: res.status, headers: Object.fromEntries(res.headers), body: text.slice(0, ${MAX_SESSION_BODY}) }
})()`
}

/** Electron's user agent without the tokens that name Electron and Vorn, which some sign-in pages refuse. */
export function plainUserAgent(agent: string): string {
  return agent
    .replace(/\s(?:Electron|vorn)\/\S+/gi, '')
    .replace(/\s{2,}/g, ' ')
    .trim()
}

function valueAt(value: unknown, path: string): unknown {
  return path
    .split('.')
    .reduce<unknown>(
      (at, key) =>
        at && typeof at === 'object' ? (at as Record<string, unknown>)[key] : undefined,
      value
    )
}

/** Who the check's answer says is signed in, from the manifest's identity fields; null when it names no one. */
export function identityFrom(body: string, paths: readonly string[]): string | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(body)
  } catch {
    return null
  }
  const values = paths
    .map((path) => valueAt(parsed, path))
    .filter(
      (value) => (typeof value === 'string' && value.trim() !== '') || typeof value === 'number'
    )
    .map(String)
  if (values.length === 0) return null
  const [first, ...rest] = values
  return rest.length > 0 ? `${first} (${rest.join(', ')})` : first!
}

/** Profile folders under userData/Partitions that belong to no connection any more. */
export function staleConnectionFolders(
  folders: readonly string[],
  connectionIds: readonly string[]
): string[] {
  const live = new Set(connectionIds.map((id) => `vorn-connection-${id}`))
  return folders.filter((name) => name.startsWith('vorn-connection-') && !live.has(name))
}
