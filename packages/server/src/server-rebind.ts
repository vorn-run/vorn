import type { Server } from 'node:http'
import { configManager } from './config-manager'
import log from './logger'

let httpServer: Server | null = null
let currentHost = '127.0.0.1'
let boundPort = 0
let rebindInFlight: Promise<void> | null = null

export function initRebind(server: Server, host: string, port: number): void {
  httpServer = server
  currentHost = host
  boundPort = port
}

export function getCurrentHost(): string {
  return currentHost
}

/**
 * Rebind if the reachability setting has changed.
 *
 * Binds 0.0.0.0 when remote access is enabled, else loopback. Tailscale used to be
 * required as well, which made the tailnet the security boundary; every connection
 * is authenticated now, so the credential is. Tailscale remains the recommended
 * way to reach the server — it is just no longer what protects it.
 */
export async function checkAndRebind(): Promise<void> {
  // Serialize: if a rebind is already running, just wait for it
  if (rebindInFlight) {
    await rebindInFlight
    return
  }

  rebindInFlight = doRebind()
  try {
    await rebindInFlight
  } finally {
    rebindInFlight = null
  }
}

async function doRebind(): Promise<void> {
  if (!httpServer) return

  const config = configManager.loadConfig()
  // No Tailscale probe here any more. It used to gate this, which also meant a
  // hung `tailscale status` stalled the rebind for its full ten-second timeout
  // with every other rebind queued behind it.
  const desiredHost = config.defaults.networkAccessEnabled ? '0.0.0.0' : '127.0.0.1'

  if (desiredHost === currentHost) return

  log.info(`[server] rebinding from ${currentHost} to ${desiredHost}:${boundPort}`)

  try {
    const server = httpServer
    if (typeof server.closeAllConnections === 'function') {
      server.closeAllConnections()
    }
    await new Promise<void>((resolve) => server.close(() => resolve()))
    await new Promise<void>((resolve, reject) => {
      const onError = (err: unknown) => {
        server.removeListener('listening', onListening)
        reject(err)
      }
      const onListening = () => {
        server.removeListener('error', onError)
        resolve()
      }
      server.once('error', onError)
      server.listen(boundPort, desiredHost, onListening)
    })
    currentHost = desiredHost
    log.info(`[server] rebound successfully to ${desiredHost}:${boundPort}`)
  } catch (err) {
    log.error({ err }, '[server] rebind failed')
  }
}
