/** Where Vorn serves a browser-sign-in connector the window it signed in through. */
export const SESSION_HOST_ENV = 'VORN_SESSION_HOST'
export const SESSION_TOKEN_ENV = 'VORN_SESSION_TOKEN'

/** The signed-in window could not make the call: Vorn is closed, too old, or not the caller. */
export class SessionUnavailableError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'SessionUnavailableError'
  }
}

export interface SessionFetchOptions {
  env?: NodeJS.ProcessEnv
  /** Replaced in tests so nothing opens a socket. */
  fetchImpl?: typeof fetch
}

/** Long enough for a window to load its origin the first time, short enough to fail a wedged app. */
const SESSION_TIMEOUT_MS = 45_000

/** The endpoint is served on this machine, so the token never leaves it. */
const LOOPBACK_HOSTS = ['127.0.0.1', 'localhost', '[::1]']

/** An origin a connector may act on: `https://host`, or `https://*.host` for every subdomain. */
export const ORIGIN_PATTERN = /^https:\/\/(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)+$/i

/** Statuses a Response must carry with no body. */
const NULL_BODY_STATUSES = new Set([101, 204, 205, 304])

/** Whether `url` is on one of the declared origins. */
export function withinOrigins(origins: readonly string[], url: string): boolean {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return false
  }
  if (parsed.protocol !== 'https:' || parsed.port !== '') return false
  const target = parsed.hostname.toLowerCase()
  return origins.some((origin) => {
    if (!ORIGIN_PATTERN.test(origin)) return false
    const wildcard = origin.startsWith('https://*.')
    const host = origin.slice(wildcard ? 'https://*.'.length : 'https://'.length).toLowerCase()
    return wildcard ? target.endsWith(`.${host}`) : target === host
  })
}

function endpoint(env: NodeJS.ProcessEnv): { url: string; token: string } {
  const url = env[SESSION_HOST_ENV]?.trim()
  const token = env[SESSION_TOKEN_ENV]?.trim()
  if (!url || !token) {
    throw new SessionUnavailableError(
      `This connector acts through a signed-in Vorn window; run it from Vorn, which sets ${SESSION_HOST_ENV}`
    )
  }
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw new SessionUnavailableError(
      `${SESSION_HOST_ENV} is ${JSON.stringify(url)}, which is not a URL`
    )
  }
  // Checked before the token is sent, so a variable pointing elsewhere cannot collect it.
  if (parsed.protocol !== 'http:' || !LOOPBACK_HOSTS.includes(parsed.hostname)) {
    throw new SessionUnavailableError(
      `${SESSION_HOST_ENV} is ${JSON.stringify(url)}; the endpoint is served on this machine, over http on ${LOOPBACK_HOSTS.join(', ')}`
    )
  }
  return { url: url.replace(/\/$/, ''), token }
}

interface SessionReply {
  status: number
  headers?: Record<string, string>
  body?: string
}

function readReply(text: string): SessionReply {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    throw new Error('The signed-in window answered with a body that is not JSON')
  }
  const reply = parsed as Partial<SessionReply>
  if (!parsed || typeof parsed !== 'object' || typeof reply.status !== 'number') {
    throw new Error('The signed-in window answered without a status')
  }
  return reply as SessionReply
}

function refusal(text: string): string | undefined {
  try {
    const error = (JSON.parse(text) as { error?: unknown }).error
    return typeof error === 'string' && error ? error : undefined
  } catch {
    return undefined
  }
}

/** A fetch whose requests run inside the connection's signed-in Vorn window, so no cookie reaches this process. */
export function createSessionFetch(options: SessionFetchOptions = {}): typeof fetch {
  const env = options.env ?? process.env
  const call = options.fetchImpl ?? fetch
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const { url, token } = endpoint(env)
    const request = new Request(input, init)
    const body = request.body ? await request.text() : undefined
    const answer = await call(`${url}/fetch`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        url: request.url,
        method: request.method,
        headers: Object.fromEntries(request.headers),
        ...(body !== undefined && { body })
      }),
      signal: AbortSignal.timeout(SESSION_TIMEOUT_MS)
    })
    const text = await answer.text()
    if (answer.status === 503) {
      throw new SessionUnavailableError(
        refusal(text) ?? 'Vorn could not reach the signed-in window'
      )
    }
    if (!answer.ok) {
      throw new Error(
        refusal(text) ?? `The signed-in window refused the call with HTTP ${answer.status}`
      )
    }
    const reply = readReply(text)
    return new Response(NULL_BODY_STATUSES.has(reply.status) ? null : (reply.body ?? ''), {
      status: reply.status,
      ...(reply.headers && { headers: reply.headers })
    })
  }) as typeof fetch
}
