// The pure half of a connection's signed-in profile, kept apart from Electron so it can be tested.

/** The largest body a signed-in call hands back, so a runaway page cannot fill memory. */
export const MAX_SESSION_BODY = 4 * 1024 * 1024
/** The largest answer a signed-in call hands back as bytes; a bigger one is refused, never cut short. */
export const MAX_SESSION_BYTES = 16 * 1024 * 1024
/** What a call over that limit is refused with, in the page and in main alike. */
export const SESSION_BYTES_REFUSAL = `The answer is over the ${MAX_SESSION_BYTES / (1024 * 1024)} MiB a signed-in call carries`

import { CONNECTION_PROFILE_PREFIX, type SessionAnswer, type SessionRequest } from '../shared/types'
import { sessionHeaders } from '@vornrun/shared/connector-origins'

export type { SessionAnswer, SessionRequest }

/** The request headers a call may set; the page supplies the rest itself, cookies included. */
export function allowedHeaders(headers: Record<string, string> = {}): Record<string, string> {
  return sessionHeaders(headers)
}

/** The script a runner page executes, built from JSON so nothing a connector sends runs as code. */
export function fetchScript(request: SessionRequest): string {
  const init = {
    method: request.method.toUpperCase(),
    headers: allowedHeaders(request.headers),
    credentials: 'include',
    ...(request.body !== undefined && { body: request.body })
  }
  // Counted as the bytes arrive, so a chunked or compressed answer is stopped at the limit too.
  const read = request.binaryBody
    ? `const refuse = (size) => { if (size > ${MAX_SESSION_BYTES}) throw new Error(${JSON.stringify(SESSION_BYTES_REFUSAL)}) }
  refuse(Number(res.headers.get('content-length')))
  const reader = res.body?.getReader()
  const parts = []
  let total = 0
  while (reader) {
    const { done, value } = await reader.read()
    if (done) break
    total += value.byteLength
    if (total > ${MAX_SESSION_BYTES}) await reader.cancel()
    refuse(total)
    parts.push(value)
  }
  const bytes = new Uint8Array(total)
  let at = 0
  for (const part of parts) {
    bytes.set(part, at)
    at += part.byteLength
  }
  const out = { body: '', bodyBase64: bytes.toBase64() }`
    : `const text = await res.text()
  const out = { body: text.slice(0, ${MAX_SESSION_BODY}) }`
  return `(async () => {
  const res = await fetch(${JSON.stringify(request.url)}, ${JSON.stringify(init)})
  ${read}
  return { status: res.status, headers: Object.fromEntries(res.headers), ...out }
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
        at && typeof at === 'object' && Object.prototype.hasOwnProperty.call(at, key)
          ? (at as Record<string, unknown>)[key]
          : undefined,
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
  const live = new Set(connectionIds.map((id) => `${CONNECTION_PROFILE_PREFIX}${id}`))
  return folders.filter((name) => name.startsWith(CONNECTION_PROFILE_PREFIX) && !live.has(name))
}
