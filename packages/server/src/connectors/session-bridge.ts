import { randomBytes } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import {
  SESSION_CALL_HEADER,
  type ActionResult,
  type SdkBrowserSignIn,
  type SessionCall,
  type SessionRequest,
  type SourceConnection
} from '@vornrun/shared/types'
import { withinOrigins } from '@vornrun/shared/connector-origins'
import { browserBridge } from '../browser-bridge'
import { dbSetConnectionSignIn, dbSignalChange } from '../database'
import { constantTimeEqual } from '../token-manager'
import { bearerFrom } from '../ws-auth'
import { isLoopbackAddress } from '../ws-handler'
import { refuse } from '../plain-refusal'
import log from '../logger'

/** One call in the window, well inside the tool call's own limit. */
const CALL_TIMEOUT_MS = 20_000
const MAX_REQUEST_BYTES = 1024 * 1024
/** Enough of one tool call's requests to explain it failing. */
const KEPT_CALLS = 50
const METHODS = new Set(['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE'])

/** What a browser connector's child was started with, kept beside the child and gone with it. */
export interface SessionGrant {
  token: string
  browser: SdkBrowserSignIn
  /** The requests of each tool call in flight, keyed by the call's own key. */
  calls: Map<string, SessionCall[]>
}

let origin = ''

export function setSessionBridgeOrigin(value: string): void {
  origin = value
}

/** A token for one child alone, and the environment that hands it the endpoint. */
export function mintSessionGrant(
  connectionId: string,
  browser: SdkBrowserSignIn
): { grant: SessionGrant; env: Record<string, string> } {
  if (!origin) throw new Error('The signed-in window endpoint has no address yet')
  const token = randomBytes(32).toString('base64url')
  return {
    grant: { token, browser, calls: new Map() },
    env: {
      VORN_BROWSER_HOST: `${origin}/connections/${connectionId}/browser`,
      VORN_BROWSER_TOKEN: token
    }
  }
}

/** A tool call whose window requests are being recorded. */
export interface OpenSessionCall {
  grant: SessionGrant
  key: string
}

/** Start recording a tool call's requests; the key travels with the call to the child and back. */
export function openSessionCall(grant: SessionGrant): OpenSessionCall {
  const key = randomBytes(12).toString('base64url')
  grant.calls.set(key, [])
  return { grant, key }
}

export function closeSessionCall({ grant, key }: OpenSessionCall): SessionCall[] {
  const calls = grant.calls.get(key) ?? []
  grant.calls.delete(key)
  return calls
}

export function markSignedOut(connectionId: string): void {
  dbSetConnectionSignIn(connectionId, null, null)
  dbSignalChange()
}

const checking = new Map<string, Promise<boolean | undefined>>()

/** Whether the window is still signed in, asked once however many calls failed together; undefined when no desktop can say. */
export function stillSignedIn(
  connectionId: string,
  browser: SdkBrowserSignIn
): Promise<boolean | undefined> {
  const inFlight = checking.get(connectionId)
  if (inFlight) return inFlight
  const check = (async () => {
    if (!browserBridge.isConnected) return undefined
    try {
      const answer = await browserBridge.request(
        'session:check',
        { connectionId, browser },
        CALL_TIMEOUT_MS
      )
      return answer.signedIn
    } catch {
      return undefined
    }
  })().finally(() => checking.delete(connectionId))
  checking.set(connectionId, check)
  return check
}

/** A browser connection's result, with its window calls attached and a failure told apart: Vorn closed, or the site signed it out. */
export async function sessionOutcome(
  conn: SourceConnection,
  call: OpenSessionCall,
  result: ActionResult
): Promise<ActionResult> {
  const sessionCalls = closeSessionCall(call)
  const withCalls = sessionCalls.length > 0 ? { ...result, sessionCalls } : result
  if (result.success) return withCalls
  if (sessionCalls.some((call) => call.status === 'app-offline')) {
    return {
      ...withCalls,
      errorKind: 'app-offline',
      error: `Open Vorn on the desktop ${conn.name} signed in on, then run this step again.`
    }
  }
  const refused = sessionCalls.some((call) => call.status === 401 || call.status === 403)
  if (refused && (await stillSignedIn(conn.id, call.grant.browser)) === false) {
    markSignedOut(conn.id)
    return {
      ...withCalls,
      errorKind: 'needs-sign-in',
      error: `${conn.name} was signed out. Sign in again, and this step runs again.`
    }
  }
  return withCalls
}

function record(grant: SessionGrant, key: string | undefined, call: SessionCall): void {
  const calls = key === undefined ? undefined : grant.calls.get(key)
  if (!calls) return
  calls.push(call)
  if (calls.length > KEPT_CALLS) calls.splice(0, calls.length - KEPT_CALLS)
}

function readRequest(body: unknown): SessionRequest | undefined {
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
export function registerSessionBridge(
  app: FastifyInstance,
  grantFor: (connectionId: string) => SessionGrant | undefined
): void {
  app.post(
    '/connections/:id/browser/fetch',
    { bodyLimit: MAX_REQUEST_BYTES },
    async (req, reply) => {
      if (!isLoopbackAddress(req.ip)) return refuse(reply, 403, 'Local machine only')
      const { id } = req.params as { id: string }
      const grant = grantFor(id)
      const token = bearerFrom(req.headers.authorization)
      if (
        !grant ||
        !token ||
        !constantTimeEqual(Buffer.from(token, 'utf8'), Buffer.from(grant.token, 'utf8'))
      ) {
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
      const header = req.headers[SESSION_CALL_HEADER]
      const key = typeof header === 'string' ? header : undefined
      const path = new URL(request.url).pathname
      if (!browserBridge.isConnected) {
        record(grant, key, { method: request.method, path, status: 'app-offline' })
        return refuse(reply, 503, 'Open Vorn on the desktop this connection signed in on')
      }
      try {
        const answer = await browserBridge.request(
          'session:fetch',
          { connectionId: id, origins: grant.browser.origins, request },
          CALL_TIMEOUT_MS
        )
        record(grant, key, { method: request.method, path, status: answer.status })
        return reply.code(200).send(answer)
      } catch (err) {
        record(grant, key, { method: request.method, path, status: 'failed' })
        return refuse(reply, 503, err instanceof Error ? err.message : String(err))
      }
    }
  )
  log.info('[connectors] signed-in window route registered')
}
