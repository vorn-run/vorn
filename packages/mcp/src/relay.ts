import type { Transport } from '@modelcontextprotocol/sdk/shared/transport.js'
import type { JSONRPCMessage } from '@modelcontextprotocol/sdk/types.js'
import type { VorndStatus } from '@vornrun/shared/types'

/**
 * The agent's side of vornd's MCP server.
 *
 * With the Native server switch on, vornd serves the tools at `/mcp` over
 * Streamable HTTP, and this process only carries messages between the agent's
 * stdio and that endpoint. The tools then run once, in vornd, for every agent,
 * instead of in a Node process per agent.
 *
 * What the TypeScript tools read from their own process -- the working
 * directory and `VORN_SESSION_ID` -- is the agent's, and vornd cannot see it,
 * so every request carries both.
 *
 * A Vorn restart is waited out: vornd is found again and the agent's handshake replayed.
 */

/** How long the server and vornd get to say whether vornd serves MCP. */
const ASK_TIMEOUT_MS = 3_000

export interface RelayDeps {
  /** `server:vornd` on the running server. */
  vorndStatus: () => Promise<VorndStatus>
  fetch: typeof fetch
}

/**
 * vornd's `/mcp`, when the running server's vornd serves MCP itself; null
 * when it does not or cannot be asked, and the TypeScript tools answer.
 */
export async function vorndMcpUrl(deps: RelayDeps): Promise<URL | null> {
  try {
    const status = await deps.vorndStatus()
    if (status.state !== 'on' || !status.nativeServer) return null
    const res = await deps.fetch(`http://127.0.0.1:${status.port}/vornd/health`, {
      signal: AbortSignal.timeout(ASK_TIMEOUT_MS)
    })
    // vornd answers 503 while the server is unreachable, with the same body.
    const health = (await res.json()) as { groups?: Record<string, { mode?: string }> }
    if (health.groups?.mcp?.mode !== 'native') return null
    return new URL(`http://127.0.0.1:${status.port}/mcp`)
  } catch {
    return null
  }
}

/** The headers every request to vornd's `/mcp` carries. */
export function relayHeaders(
  credential: string,
  cwd: string,
  env: NodeJS.ProcessEnv
): Record<string, string> {
  const headers: Record<string, string> = {
    Authorization: `Bearer ${credential}`,
    'Vorn-Cwd': encodeURIComponent(cwd)
  }
  const session = env.VORN_SESSION_ID
  if (session) headers['Vorn-Session-Id'] = session
  return headers
}

/** Where vornd's `/mcp` is and what each request to it carries. */
export interface Upstream {
  url: URL
  headers: Record<string, string>
}

/** How long and how often to look for vornd after it went away. */
export interface Backoff {
  firstMs: number
  maxMs: number
  /** After this, what waited is failed and the next request starts waiting again. */
  giveUpAfterMs: number
}

export const BACKOFF: Backoff = { firstMs: 200, maxMs: 5_000, giveUpAfterMs: 60_000 }

export interface RelayOptions {
  /** Finds vornd again: null while it is not serving MCP, or throws. */
  locate: () => Promise<Upstream | null>
  fetch: typeof fetch
  backoff?: Backoff
}

/** How many times one message is sent again after reconnecting before it is failed. */
const MAX_RESENDS = 3
/** How long ending the session may take. */
const DELETE_TIMEOUT_MS = 5_000
const SESSION_HEADER = 'mcp-session-id'
const PROTOCOL_HEADER = 'mcp-protocol-version'

/** `unsent` never reached vornd's tools; `broken` may have; `final` is vornd's own answer. */
type Failure = { kind: 'unsent' | 'broken' | 'final'; reason: string }

interface Session {
  id?: string
  protocol?: string
}

type Request = JSONRPCMessage & { id: string | number; method: string }

function isRequest(message: JSONRPCMessage): message is Request {
  return 'method' in message && 'id' in message
}

function methodOf(message: JSONRPCMessage): string | undefined {
  return 'method' in message ? message.method : undefined
}

/** Connection errors that mean the request was never written. */
const NOT_CONNECTED = new Set([
  'ECONNREFUSED',
  'EHOSTUNREACH',
  'ENETUNREACH',
  'EADDRNOTAVAIL',
  'ENOTFOUND',
  'UND_ERR_CONNECT_TIMEOUT'
])

function errorCode(err: unknown): string | undefined {
  for (let e: unknown = err; e instanceof Error; e = e.cause) {
    const code = (e as { code?: unknown }).code
    if (typeof code === 'string') return code
  }
  return undefined
}

function describe(err: unknown): string {
  const code = errorCode(err)
  const message = err instanceof Error ? err.message : String(err)
  return code ? `${message} (${code})` : message
}

/** The JSON-RPC messages in a body, whether JSON or a stream of events. */
function messagesIn(body: string, contentType: string | null): JSONRPCMessage[] {
  const texts: string[] = []
  if (contentType?.includes('text/event-stream')) {
    for (const event of body.split(/\r?\n\r?\n/)) {
      const data = event
        .split(/\r?\n/)
        .filter((line) => line.startsWith('data:'))
        .map((line) => line.slice(5).replace(/^ /, ''))
      if (data.length > 0) texts.push(data.join('\n'))
    }
  } else if (body.trim()) {
    texts.push(body)
  }
  return texts.flatMap((text) => {
    const parsed = JSON.parse(text) as JSONRPCMessage | JSONRPCMessage[]
    return Array.isArray(parsed) ? parsed : [parsed]
  })
}

/** Carries messages between `local` (the agent's stdio) and vornd until the agent closes its end. */
export async function relay(
  local: Transport,
  first: Upstream,
  options: RelayOptions
): Promise<void> {
  const backoff = options.backoff ?? BACKOFF
  let upstream = first
  let session: Session = {}
  /** The agent's handshake, replayed to open a session after a restart. */
  let initialize: JSONRPCMessage | undefined
  let initialized: JSONRPCMessage | undefined
  /** Whether vornd has accepted anything: a refusal before that is not a restart. */
  let reached = false
  /** Counts finished reconnects, so one restart is handled once. */
  let epoch = 0
  let lastReconnect: string | null = null
  let reconnecting: Promise<string | null> | null = null
  let closing = false

  const post = (to: Upstream, message: JSONRPCMessage, carrying: Session): Promise<Response> => {
    const headers: Record<string, string> = {
      ...to.headers,
      Accept: 'application/json, text/event-stream',
      'Content-Type': 'application/json'
    }
    if (carrying.id) headers[SESSION_HEADER] = carrying.id
    if (carrying.protocol) headers[PROTOCOL_HEADER] = carrying.protocol
    return options.fetch(to.url, { method: 'POST', headers, body: JSON.stringify(message) })
  }

  /** Asks where vornd is and, if the agent had opened a session, opens one there as it did. */
  const reopen = async (): Promise<{ upstream: Upstream; session: Session }> => {
    const found = await options.locate()
    if (!found) throw new Error('vornd is not serving MCP')
    if (!initialize) return { upstream: found, session: {} }
    const res = await post(found, initialize, {})
    if (!res.ok) throw new Error(`vornd answered ${res.status} to initialize`)
    const fresh: Session = { id: res.headers.get(SESSION_HEADER) ?? undefined }
    for (const answer of messagesIn(await res.text(), res.headers.get('content-type'))) {
      if ('error' in answer)
        throw new Error(`vornd refused initialize: ${JSON.stringify(answer.error)}`)
      const version = (answer as { result?: { protocolVersion?: unknown } }).result?.protocolVersion
      if (typeof version === 'string') fresh.protocol = version
    }
    if (initialized) {
      const note = await post(found, initialized, fresh)
      await note.body?.cancel()
      if (!note.ok) throw new Error(`vornd answered ${note.status} to notifications/initialized`)
    }
    return { upstream: found, session: fresh }
  }

  /** Tries to reopen the session until it works or the backoff gives up; null when it worked. */
  const waitForVornd = async (): Promise<string | null> => {
    const deadline = Date.now() + backoff.giveUpAfterMs
    let delay = backoff.firstMs
    for (;;) {
      try {
        const opened = await reopen()
        upstream = opened.upstream
        session = opened.session
        console.error(`reconnected to vornd at ${upstream.url.host}`)
        return null
      } catch (err) {
        const now = Date.now()
        if (now >= deadline || closing) {
          return `vornd did not come back within ${backoff.giveUpAfterMs / 1000}s: ${describe(err)}`
        }
        await new Promise((resolve) => setTimeout(resolve, Math.min(delay, deadline - now)))
        delay = Math.min(delay * 2, backoff.maxMs)
      }
    }
  }

  /** Reopens the session unless a reconnect finished since attempt `seen`; null when vornd is back. */
  const reconnect = (seen: number): Promise<string | null> => {
    if (seen !== epoch) return Promise.resolve(lastReconnect)
    reconnecting ??= waitForVornd().then((outcome) => {
      epoch++
      lastReconnect = outcome
      reconnecting = null
      return outcome
    })
    return reconnecting
  }

  /** One POST of `message`; answers go to the agent. */
  const exchange = async (message: JSONRPCMessage): Promise<Failure | null> => {
    const opening = methodOf(message) === 'initialize'
    // An `initialize` opens a new session, whatever the relay held.
    const sent = opening ? {} : session
    const to = upstream
    let res: Response
    try {
      res = await post(to, message, sent)
    } catch (err) {
      const kind = NOT_CONNECTED.has(errorCode(err) ?? '') ? 'unsent' : 'broken'
      return { kind, reason: describe(err) }
    }
    const status = res.status
    if (status === 404 && sent.id) {
      await res.body?.cancel()
      return { kind: 'unsent', reason: 'vornd no longer knows this MCP session; Vorn restarted' }
    }
    // After a restart vornd may lack MCP or a credential for a while; before one, a refusal stands.
    if (status === 503 || (reached && (status === 404 || status === 401))) {
      await res.body?.cancel()
      return { kind: 'unsent', reason: `vornd at ${to.url.host} answered ${status}` }
    }
    let body: string
    try {
      body = await res.text()
    } catch (err) {
      return { kind: 'broken', reason: `vornd's answer broke off: ${describe(err)}` }
    }
    if (!res.ok) {
      const detail = body.trim() ? `: ${body.trim()}` : ''
      return { kind: 'final', reason: `vornd answered ${status}${detail}` }
    }
    reached = true
    if (opening) {
      session = { id: res.headers.get(SESSION_HEADER) ?? undefined }
    }
    let answers: JSONRPCMessage[]
    try {
      answers = messagesIn(body, res.headers.get('content-type'))
    } catch (err) {
      return { kind: 'final', reason: `vornd's answer is not JSON-RPC: ${describe(err)}` }
    }
    for (const answer of answers) {
      if (opening) {
        const version = (answer as { result?: { protocolVersion?: unknown } }).result
          ?.protocolVersion
        if (typeof version === 'string') session = { ...session, protocol: version }
      }
      await local.send(answer).catch(() => {})
    }
    return null
  }

  /** Relays `message`, reconnecting and sending it again while vornd has not taken it. */
  const deliver = async (message: JSONRPCMessage): Promise<string | null> => {
    const method = methodOf(message)
    for (let resends = 0; ; resends++) {
      if (reconnecting) await reconnecting
      const seen = epoch
      const failure = await exchange(message)
      if (!failure) {
        if (method === 'initialize') initialize = message
        else if (method === 'notifications/initialized') initialized = message
        return null
      }
      if (failure.kind === 'final') return failure.reason
      if (failure.kind === 'broken' && method === 'tools/call') return failure.reason
      if (closing) return failure.reason
      if (resends === MAX_RESENDS) {
        return `vornd kept turning the request away after reconnecting: ${failure.reason}`
      }
      console.error(`${failure.reason}; waiting for vornd`)
      const gaveUp = await reconnect(seen)
      if (gaveUp) return gaveUp
    }
  }

  const closed = new Promise<void>((resolve) => {
    local.onclose = () => {
      closing = true
      resolve()
    }
  })
  local.onmessage = (message) => {
    void deliver(message).then((reason) => {
      if (reason === null || !isRequest(message)) return
      local
        .send({
          jsonrpc: '2.0',
          id: message.id,
          error: { code: -32603, message: `vornd's MCP server did not answer: ${reason}` }
        })
        .catch(() => {})
    })
  }
  await local.start()
  await closed
  if (session.id) {
    await options
      .fetch(upstream.url, {
        method: 'DELETE',
        headers: { ...upstream.headers, [SESSION_HEADER]: session.id },
        signal: AbortSignal.timeout(DELETE_TIMEOUT_MS)
      })
      .then((res) => res.body?.cancel())
      .catch(() => {})
  }
}
