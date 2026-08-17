import type { Server } from 'node:http'
import { getTailscaleStatus } from './tailscale'
import { configManager } from './config-manager'
import log from './logger'

let httpServer: Server | null = null
let currentHost = '127.0.0.1'
let boundPort = 0
let rebindInFlight: Promise<void> | null = null

/**
 * Hosts the web client can legitimately be served from, beyond loopback.
 *
 * Lives here rather than beside the socket route because reachability and origin
 * policy are one decision: this module is what flips the bind between loopback
 * and 0.0.0.0, at runtime, whenever the setting changes. Computed anywhere else
 * it goes stale the first time someone enables remote access without restarting
 * — and a stale list refuses the tailnet web client with a 403 on the upgrade,
 * which is the one deployment the authentication work exists to make safe.
 */
let reachableHosts: string[] = []

export function initRebind(server: Server, host: string, port: number, hosts: string[]): void {
  httpServer = server
  currentHost = host
  boundPort = port
  reachableHosts = hosts
}

/**
 * Whether an upgrade carrying this `Origin` may proceed.
 *
 * An absent Origin is allowed: non-browser clients (the desktop bridge, MCP) do
 * not send one, and per OWASP the header is only meaningful for browsers. Those
 * clients are still held to the credential check, which is the control that
 * actually applies to them.
 *
 * Exact comparison against an explicit set — no wildcards and no substring
 * matching, both of which are the usual way an allowlist is defeated. `http://`
 * is included because the app is reachable over a tailnet without TLS, which is
 * why `packages/web/src/main.tsx` polyfills `crypto.randomUUID` for a non-secure
 * context; requiring https would lock out that deployment.
 */
export function isAllowedOrigin(origin: string | undefined): boolean {
  if (origin === undefined) return true
  return ['127.0.0.1', 'localhost', '[::1]', ...reachableHosts].some(
    (host) => origin === `http://${host}:${boundPort}` || origin === `https://${host}:${boundPort}`
  )
}

export function getCurrentHost(): string {
  return currentHost
}

/**
 * Check if the server needs to rebind based on current config + Tailscale state.
 * Binds to 0.0.0.0 when networkAccessEnabled AND Tailscale is running, else 127.0.0.1.
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
  let desiredHost = '127.0.0.1'

  if (config.defaults.networkAccessEnabled) {
    try {
      const tsStatus = await getTailscaleStatus()
      if (tsStatus.running) {
        desiredHost = '0.0.0.0'
        // Refreshed on the same transition that changes reachability, so the
        // origin policy cannot disagree with what the server is bound to.
        reachableHosts = [tsStatus.selfIP, tsStatus.selfDNSName].filter(
          (h): h is string => typeof h === 'string' && h.length > 0
        )
      } else {
        reachableHosts = []
      }
    } catch {
      // Tailscale check failed, stay on localhost
    }
  } else {
    reachableHosts = []
  }

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
