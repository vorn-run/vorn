import WebSocket, { type RawData } from 'ws'
import type { IncomingHttpHeaders } from 'node:http'
import { isLoopbackAddress } from './ws-handler'
import log from './logger'

/**
 * A client on another machine, put through vornd.
 *
 * vornd listens only on loopback, and every terminal is in it, so a phone or a
 * browser on the network that reaches this server directly would see its
 * sessions but none of their output. Its socket is carried to vornd instead,
 * whole, as if it had connected there: vornd answers its terminal calls and
 * hands everything else back to this server on a second socket, which is
 * handled like any other. The address it came from rides along in
 * `VORN_PEER_HEADER`, so that socket is still judged as a stranger's.
 */

/** The address a relayed client connected from, set by the relay alone. */
export const VORN_PEER_HEADER = 'x-vorn-peer'

/** What the client's upgrade said that the server judges it by. */
const FORWARDED = ['authorization', 'origin', 'host', 'cookie', 'user-agent'] as const

/**
 * Where a socket really came from: a relayed one names its client's address,
 * which only a peer on this machine may say. Anyone else gets its own.
 */
export function peerAddress(
  remoteAddress: string | undefined,
  headers: IncomingHttpHeaders
): string | undefined {
  const said = headers[VORN_PEER_HEADER]
  return isLoopbackAddress(remoteAddress) && typeof said === 'string' && said ? said : remoteAddress
}

/** Whether a socket from `remoteAddress` goes through vornd: one from another machine. */
export function relaysThroughVornd(
  remoteAddress: string | undefined,
  vorndPort: number | null
): vorndPort is number {
  return vorndPort !== null && !!remoteAddress && !isLoopbackAddress(remoteAddress)
}

/** A close code that may be sent on: the reserved ones only describe a close. */
function sendable(code: number): boolean {
  return (code >= 1000 && code <= 1003) || (code >= 1007 && code <= 1014) || code >= 3000
}

function closeWith(ws: WebSocket, code: number, reason: Buffer | string): void {
  if (ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) return
  if (sendable(code)) ws.close(code, reason.toString().slice(0, 120))
  else ws.close()
}

/**
 * Carry `client` to vornd on `vorndPort`. `direct` serves it here instead when
 * vornd cannot be reached; what it sent meanwhile is handed over in order.
 */
export function relayThroughVornd(
  client: WebSocket,
  request: { url?: string; headers: IncomingHttpHeaders },
  vorndPort: number,
  from: string,
  direct: () => void
): void {
  const headers: Record<string, string> = { [VORN_PEER_HEADER]: from }
  for (const name of FORWARDED) {
    const value = request.headers[name]
    if (typeof value === 'string') headers[name] = value
  }
  const held: Array<{ data: RawData; isBinary: boolean }> = []
  let opened = false
  let fellBack = false

  const upstream = new WebSocket(`ws://127.0.0.1:${vorndPort}${request.url ?? '/ws'}`, {
    headers,
    perMessageDeflate: false,
    maxPayload: 64 * 1024 * 1024
  })

  const fromClient = (data: RawData, isBinary: boolean): void => {
    if (!opened) held.push({ data, isBinary })
    else upstream.send(data, { binary: isBinary })
  }
  client.on('message', fromClient)

  const fallBack = (why: string): void => {
    if (opened || fellBack) return
    fellBack = true
    log.warn(`[vornd] could not put a client through vornd (${why}); serving it here`)
    client.off('message', fromClient)
    upstream.removeAllListeners()
    upstream.on('error', () => {})
    upstream.terminate()
    if (client.readyState !== WebSocket.OPEN) return
    direct()
    for (const { data, isBinary } of held.splice(0)) client.emit('message', data, isBinary)
  }

  upstream.once('open', () => {
    if (fellBack) return
    opened = true
    for (const { data, isBinary } of held.splice(0)) upstream.send(data, { binary: isBinary })
  })
  // The server's refusal comes back as the close vornd passes on; an answer
  // that is not an upgrade at all means vornd is not there to ask.
  upstream.once('unexpected-response', (_req, res) => {
    if (res.statusCode === 401 || res.statusCode === 403) {
      opened = true
      closeWith(client, 1008, res.statusCode === 401 ? 'unauthorized' : 'forbidden')
      upstream.terminate()
      return
    }
    fallBack(`it answered ${res.statusCode}`)
  })
  upstream.on('error', (err) => {
    if (!opened) fallBack(err.message)
    else closeWith(client, 1011, 'vornd went away')
  })
  upstream.on('message', (data, isBinary) => {
    if (client.readyState === WebSocket.OPEN) client.send(data, { binary: isBinary })
  })
  upstream.on('close', (code, reason) => {
    if (fellBack) return
    closeWith(client, code, reason)
  })
  client.on('close', (code, reason) => {
    if (fellBack) return
    if (upstream.readyState === WebSocket.CONNECTING) upstream.terminate()
    else closeWith(upstream, code, reason)
  })
  client.on('error', () => upstream.terminate())
}
