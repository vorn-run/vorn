import { loopbackEndpoint } from './loopback'

/** Where Vorn serves a browser-sign-in connector the window it signed in through. */
export const BROWSER_HOST_ENV = 'VORN_BROWSER_HOST'
export const BROWSER_TOKEN_ENV = 'VORN_BROWSER_TOKEN'
/** The tool call a window request belongs to, so Vorn can tell a step's own requests from another's. */
export const SESSION_CALL_META = 'vorn/sessionCall'
export const SESSION_CALL_HEADER = 'x-vorn-session-call'

/** The signed-in window could not make the call: Vorn is closed, too old, or not the caller. */
export class SessionUnavailableError extends Error {
  /** Asking again cannot bring the window back, so the SDK's retries let this through at once. */
  readonly retryable = false
  constructor(message: string) {
    super(message)
    this.name = 'SessionUnavailableError'
  }
}

/** Vorn refused the call itself, for instance because it is off the connector's origins. */
export class SessionRefusedError extends Error {
  readonly retryable = false
  constructor(message: string) {
    super(message)
    this.name = 'SessionRefusedError'
  }
}

export interface SessionFetchOptions {
  env?: NodeJS.ProcessEnv
  /** Replaced in tests so nothing opens a socket. */
  fetchImpl?: typeof fetch
  /** The key of the tool call these requests belong to, from its MCP metadata. */
  call?: string
}

/** Long enough for a window to load its origin the first time, short enough to fail a wedged app. */
const SESSION_TIMEOUT_MS = 45_000

/** Statuses a Response must carry with no body. */
const NULL_BODY_STATUSES = new Set([204, 205, 304])

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

const refusal = (text: string): string | undefined => text.trim() || undefined

/** A fetch whose requests run inside the connection's signed-in Vorn window, so no cookie reaches this process. */
export function createSessionFetch(options: SessionFetchOptions = {}): typeof fetch {
  const env = options.env ?? process.env
  const call = options.fetchImpl ?? fetch
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const { url, token } = loopbackEndpoint(
      env,
      {
        urlVar: BROWSER_HOST_ENV,
        tokenVar: BROWSER_TOKEN_ENV,
        missing: `This connector acts through a signed-in Vorn window; run it from Vorn, which sets ${BROWSER_HOST_ENV}`,
        served: 'the endpoint is served on this machine'
      },
      (message) => new SessionUnavailableError(message)
    )
    const request = new Request(input, init)
    // Carried as text both ways: signed-in calls are JSON and form posts, not uploads.
    const body = request.body ? await request.text() : undefined
    const answer = await call(`${url}/fetch`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${token}`,
        'content-type': 'application/json',
        ...(options.call && { [SESSION_CALL_HEADER]: options.call })
      },
      body: JSON.stringify({
        url: request.url,
        method: request.method,
        headers: Object.fromEntries(request.headers),
        ...(body !== undefined && { body })
      }),
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(SESSION_TIMEOUT_MS)])
    })
    const text = await answer.text()
    if (answer.status === 503) {
      throw new SessionUnavailableError(
        refusal(text) ?? 'Vorn could not reach the signed-in window'
      )
    }
    if (!answer.ok) {
      throw new SessionRefusedError(
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
