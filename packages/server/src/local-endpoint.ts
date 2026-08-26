import http from 'http'
import fs from 'fs'
import { WebSocketServer } from 'ws'
import log from './logger'
import { handleConnection } from './ws-handler'
import { parseTopics } from './broadcast'
import { bearerFrom } from './ws-auth'
import {
  endpointPath,
  scratchPathFor,
  canHostEndpoint,
  claimEndpoint,
  abandonScratch
} from './endpoint'

/**
 * The listener behind the canonical endpoint.
 *
 * Deliberately narrow: a WebSocket upgrade and nothing else. It would be less
 * code to hand this server fastify's `app.routing` and get every route for free,
 * and it would be wrong twice over.
 *
 * `/api/pair/redeem` caps attempts per client address (`index.ts`), and a unix
 * peer has no address — every caller would bucket under one empty string and the
 * cap would collapse to a single shared allowance. And `@fastify/websocket`
 * attaches to one server, `wssOptions.server || fastify.server`; pointing it here
 * would move `/ws` off the TCP listener and cut off every web and phone client.
 *
 * So HTTP stays on TCP, and this carries the one thing that has to be reachable
 * by name rather than by port: a desktop on this machine, asking who is serving.
 */

/**
 * What came of trying to hold this machine's endpoint.
 *
 * Three outcomes, and conflating the last two is a bug with teeth. `unavailable`
 * means no endpoint was possible here at all -- win32, a directory anyone can
 * write, a path too long -- and the server carries on serving over TCP, because
 * a machine with no socket is still a machine that needs a server. `lost` means
 * the endpoint exists and belongs to somebody else, and this process is a second
 * server on one database: `saveSessions` is a whole-table replace, so two of them
 * erase each other's work.
 */
export type EndpointOutcome =
  | { kind: 'held'; endpoint: LocalEndpoint }
  | { kind: 'unavailable'; why: string }
  | { kind: 'lost'; because: string }

export interface LocalEndpoint {
  /** The canonical path, once held. */
  readonly path: string
  /** Whether this process still holds the name. Read, never cached. */
  holds(): boolean
  close(): Promise<void>
}

/**
 * Bring up the endpoint and claim its name, or explain why not.
 *
 * Never throws and never blocks startup. Every failure is a downgrade to
 * TCP-only, because a server nobody can reach is worse than a race nobody has
 * lost yet.
 */
export async function openLocalEndpoint(
  dataDir: string,
  onUpgraded: () => void
): Promise<EndpointOutcome> {
  const allowed = canHostEndpoint(dataDir)
  if (!allowed.ok) {
    log.info({ why: allowed.why }, '[endpoint] not hosting a local socket')
    return { kind: 'unavailable', why: allowed.why ?? 'unknown' }
  }

  const canonical = endpointPath(dataDir)
  const scratch = scratchPathFor(canonical)
  let mine: { dev: number; ino: number } | null = null

  const server = http.createServer((_req, res) => {
    // Nothing here answers HTTP. Said plainly rather than left to 404, so a
    // future caller finds this comment instead of a mystery.
    res.writeHead(404)
    res.end()
  })
  const wss = new WebSocketServer({ noServer: true })

  server.on('upgrade', (req, socket, head) => {
    // Exactly `/ws`, with or without a query. `startsWith` accepted `/ws-anything`
    // too, which is a wider door than this listener means to open -- it exists to
    // carry one route.
    const route = new URL(req.url ?? '/', 'ws://endpoint').pathname
    if (route !== '/ws') {
      socket.destroy()
      return
    }
    wss.handleUpgrade(req, socket, head, (ws) => {
      handleConnection(ws, bearerFrom(req.headers.authorization), parseTopics(undefined), {
        transport: 'unix'
      })
      onUpgraded()
    })
  })

  try {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject)
      server.listen(scratch, resolve)
    })
    // Belt and braces for linux, where a socket's own mode is enforced on
    // connect. Darwin ignores it and relies on the directory, which
    // `canHostEndpoint` has already checked.
    fs.chmodSync(scratch, 0o600)
    // Taken here and never again. `server.address()` reports the path this
    // listener bound, and that name is gone the moment the claim succeeds -- so
    // the inode has to be learned while the scratch name still exists. It cannot
    // change afterwards: the listener is bound for the life of the process.
    const bound = fs.lstatSync(scratch)
    mine = { dev: bound.dev, ino: bound.ino }
  } catch (err) {
    log.warn({ err }, '[endpoint] could not bind the local socket; continuing on TCP alone')
    abandonScratch(scratch)
    return { kind: 'unavailable', why: (err as Error).message }
  }

  const outcome = await claimEndpoint(scratch, canonical)
  if (!outcome.held) {
    log.info({ because: outcome.because }, '[endpoint] another server holds this machine')
    // Closing unlinks the scratch name, which is ours. The canonical entry is
    // untouched, because libuv only knows the path this server bound.
    await new Promise<void>((resolve) => server.close(() => resolve()))
    abandonScratch(scratch)
    return { kind: 'lost', because: outcome.because }
  }

  log.info({ path: canonical }, '[endpoint] holding the local endpoint')

  const endpoint: LocalEndpoint = {
    path: canonical,
    holds: () => stillOurs(canonical, mine),
    close: async () => {
      // Terminated, not asked. `ws` with `clientTracking` does not close tracked
      // sockets on `close()` -- it waits for them, and so does the http server
      // behind it. A half-open client, which is exactly what the idle watch's
      // duration clock exists to tolerate, would hold this promise for ever:
      // `shutdown()` would stall to its deadline and leave on exit(1), read by
      // the launcher as a crash, after `killAll()` had already run.
      for (const client of wss.clients) client.terminate()
      server.closeAllConnections()
      await new Promise<void>((resolve) => {
        wss.close(() => resolve())
      })
      await new Promise<void>((resolve) => server.close(() => resolve()))
    }
  }
  return { kind: 'held', endpoint }
}

/**
 * Whether the canonical entry still resolves to this server's listener.
 *
 * Asked rather than remembered, because the answer can change without this
 * process being told: another server that found this one unreachable is entitled
 * to take the name, and the only honest way to know is to look. The inode is the
 * identity -- a path comparison would say yes to somebody else's socket sitting
 * at the same name.
 */
function stillOurs(canonical: string, mine: { dev: number; ino: number } | null): boolean {
  if (!mine) return false
  try {
    const now = fs.lstatSync(canonical)
    return now.isSocket() && now.ino === mine.ino && now.dev === mine.dev
  } catch {
    return false
  }
}
