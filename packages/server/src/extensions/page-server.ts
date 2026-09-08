import Fastify, { type FastifyInstance } from 'fastify'
import { registerExtensionPages, type ExtensionRouteDeps } from './bridge'
import log from '../logger'

/**
 * The origin a pane's page is served from, which is deliberately not the app's.
 *
 * A page and the web client on one origin share a browser's storage and may
 * open each other's sockets, so a page that loads a hostile script would hold
 * whatever the client holds — the permission list would mean nothing. Its own
 * port costs one listener and takes that away: this origin serves pages and the
 * bridge those pages call, and answers nothing else.
 */

interface PageServer {
  origin: string
  close: () => Promise<void>
}

let running: PageServer | undefined

export function extensionPageOrigin(): string {
  return running?.origin ?? ''
}

export async function startExtensionPageServer(deps: ExtensionRouteDeps): Promise<PageServer> {
  if (running) return running
  const app: FastifyInstance = Fastify({ logger: false })
  registerExtensionPages(app, deps)
  // Ephemeral and loopback: nothing off this machine may reach a page.
  await app.listen({ host: '127.0.0.1', port: 0 })
  const address = app.server.address()
  const port = typeof address === 'object' && address ? address.port : 0
  const server: PageServer = {
    origin: `http://127.0.0.1:${port}`,
    close: async () => {
      if (running === server) running = undefined
      await app.close()
    }
  }
  running = server
  log.info(`[extensions] pane pages served on ${server.origin}`)
  return server
}

export async function stopExtensionPageServer(): Promise<void> {
  await running?.close()
}
