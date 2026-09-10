import { randomBytes, timingSafeEqual } from 'node:crypto'
import type { FastifyInstance, FastifyReply } from 'fastify'
import type { SdkBrowserSignIn, SessionCall } from '@vornrun/shared/types'
import { withinOrigins } from '@vornrun/shared/connector-origins'
import { browserBridge } from '../browser-bridge'
import { isLoopbackAddress } from '../ws-handler'
import log from '../logger'

/** One call in the window, well inside the tool call's own limit. */
const CALL_TIMEOUT_MS = 20_000
const MAX_REQUEST_BYTES = 1024 * 1024
/** Enough recent calls to explain a failed step. */
const KEPT_CALLS = 50
const METHODS = new Set(['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE'])

interface Grant {
  token: string
  browser: SdkBrowserSignIn
  calls: Array<SessionCall & { at: number }>
}

/** The connections whose child may call through a window, keyed by connection id. */
const grants = new Map<string, Grant>()
let origin = ''

export function setSessionBridgeOrigin(value: string): void {
  origin = value
}

/** The endpoint and token a browser connector's child starts with; each spawn replaces the last token. */
export function sessionEnvFor(
  connectionId: string,
  browser: SdkBrowserSignIn
): Record<string, string> {
  if (!origin) throw new Error('The signed-in window endpoint has no address yet')
  const token = randomBytes(32).toString('base64url')
  grants.set(connectionId, { token, browser, calls: [] })
  return {
    VORN_BROWSER_HOST: `${origin}/connections/${connectionId}/browser`,
    VORN_BROWSER_TOKEN: token
  }
}

export function browserSignInFor(connectionId: string): SdkBrowserSignIn | undefined {
  return grants.get(connectionId)?.browser
}

export function forgetSessionGrant(connectionId: string): void {
  grants.delete(connectionId)
}

/** The calls a connection's child made through its window since `since`. */
export function sessionCallsSince(connectionId: string, since: number): SessionCall[] {
  return (grants.get(connectionId)?.calls ?? [])
    .filter((call) => call.at >= since)
    .map(({ method, path, status }) => ({ method, path, status }))
}

/** Whether the connection's window is still signed in, asked of the desktop that holds it; undefined when it cannot say. */
export async function stillSignedIn(connectionId: string): Promise<boolean | undefined> {
  const grant = grants.get(connectionId)
  if (!grant || !browserBridge.isConnected) return undefined
  try {
    const answer = await browserBridge.request(
      'session:check',
      { connectionId, browser: grant.browser },
      CALL_TIMEOUT_MS
    )
    return answer.signedIn
  } catch {
    return undefined
  }
}

function record(grant: Grant, call: SessionCall): void {
  grant.calls.push({ ...call, at: Date.now() })
  if (grant.calls.length > KEPT_CALLS) grant.calls.splice(0, grant.calls.length - KEPT_CALLS)
}

function refuse(reply: FastifyReply, code: number, error: string): FastifyReply {
  return reply.code(code).send({ error })
}

function sameToken(given: string, expected: string): boolean {
  const a = Buffer.from(given)
  const b = Buffer.from(expected)
  return a.length === b.length && timingSafeEqual(a, b)
}

interface CallRequest {
  url: string
  method: string
  headers?: Record<string, string>
  body?: string
}

function readRequest(body: unknown): CallRequest | undefined {
  if (!body || typeof body !== 'object') return undefined
  const value = body as Record<string, unknown>
  if (typeof value.url !== 'string' || typeof value.method !== 'string') return undefined
  if (value.body !== undefined && typeof value.body !== 'string') return undefined
  const headers =
    value.headers && typeof value.headers === 'object'
      ? Object.fromEntries(
          Object.entries(value.headers as Record<string, unknown>).filter(
            (entry): entry is [string, string] => typeof entry[1] === 'string'
          )
        )
      : undefined
  return {
    url: value.url,
    method: value.method.toUpperCase(),
    ...(headers && { headers }),
    ...(value.body !== undefined && { body: value.body as string })
  }
}

/** A browser connector's child, calling through the window its connection signed in on. */
export function registerSessionBridge(app: FastifyInstance): void {
  app.post(
    '/connections/:id/browser/fetch',
    { bodyLimit: MAX_REQUEST_BYTES },
    async (req, reply) => {
      if (!isLoopbackAddress(req.ip)) return refuse(reply, 403, 'Local machine only')
      const { id } = req.params as { id: string }
      const grant = grants.get(id)
      const auth = req.headers.authorization
      const token = typeof auth === 'string' && auth.startsWith('Bearer ') ? auth.slice(7) : ''
      if (!grant || !token || !sameToken(token, grant.token)) {
        return refuse(reply, 401, 'This endpoint does not know that caller')
      }
      const request = readRequest(req.body)
      if (!request) return refuse(reply, 400, 'Send { url, method, headers?, body? }')
      if (!METHODS.has(request.method)) {
        return refuse(reply, 405, `${request.method} is not a method a signed-in call may use`)
      }
      if (!withinOrigins(grant.browser.origins, request.url)) {
        return refuse(reply, 403, `${request.url} is not on one of this connection's origins`)
      }
      const path = new URL(request.url).pathname
      if (!browserBridge.isConnected) {
        record(grant, { method: request.method, path, status: 'app-offline' })
        return refuse(reply, 503, 'Open Vorn on the desktop this connection signed in on')
      }
      try {
        const answer = await browserBridge.request(
          'session:fetch',
          { connectionId: id, origins: grant.browser.origins, request },
          CALL_TIMEOUT_MS
        )
        record(grant, { method: request.method, path, status: answer.status })
        return reply.code(200).send(answer)
      } catch (err) {
        record(grant, { method: request.method, path, status: 'failed' })
        return refuse(reply, 503, err instanceof Error ? err.message : String(err))
      }
    }
  )
  log.info('[connectors] signed-in window route registered')
}
