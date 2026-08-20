import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import Fastify from 'fastify'
import type { FastifyReply, FastifyRequest } from 'fastify'

// CJS (prod bundle): __dirname is a global. ESM (dev/tsx): fall back to the
// directory of the entry script. Avoids import.meta.url which tsup emits as
// import_meta.url in the CJS bundle, crashing at runtime.
const _dirname = typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
import websocket from '@fastify/websocket'
import fastifyStatic from '@fastify/static'
import { handleConnection, registerMethod } from './ws-handler'
import { parseTopics, clientRegistry } from './broadcast'
import { IPC } from '@vornrun/shared/types'
import { registerAllMethods, setServerPort } from './register-methods'
import { configManager } from './config-manager'
import { initBootstrapSecret, clearLocalCredential, bearerFrom } from './ws-auth'
import { getDataDir } from './database'
import { parseServerArgs } from './server-args'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { scheduler } from './scheduler'
import { getTaskImagePath as resolveTaskImagePath } from './task-images'
import { redeemCode, pollRequest, pendingRequests } from './pairing'
import { getTailscaleStatus } from './tailscale'
import { initRebind, checkAndRebind } from './server-rebind'
import { isAllowedUpgrade, logRefusedUpgrade, setTrustedOriginHosts } from './ws-origin'
import { setEnvPassthrough, setLaunchDataDir } from './process-utils'
import log from './logger'

/**
 * Names, beyond IP literals and `localhost`, that the web client may legitimately
 * be served from.
 *
 * Best-effort by design. A refused name is a fallback rather than a lockout — a
 * tailnet client still connects by its `100.x` literal — so this must never block
 * startup or a config change on a Tailscale probe that may be slow or absent.
 */
async function refreshTrustedOrigins(): Promise<void> {
  try {
    const status = await getTailscaleStatus()
    setTrustedOriginHosts(status.running ? [status.selfIP, status.selfDNSName].filter(Boolean) : [])
  } catch {
    setTrustedOriginHosts([])
  }
}

export async function startServer(
  options: { host?: string; port?: number; dataDir?: string } = {}
) {
  // Initialize database + config. This resolves the data directory for the whole
  // process; everything else reads it back with getDataDir() rather than
  // deriving it again, so nothing can disagree about where the files are.
  configManager.init(options.dataDir)
  configManager.watchDb()
  const dataDir = getDataDir()
  // Hand it to the spawn path, which cannot import the database module to ask.
  // Anything launched from a session inherits VORN_DATA_DIR and can then find the
  // port and credential files even when --data-dir moved them.
  setLaunchDataDir(dataDir)

  // Register built-in connectors
  const { connectorRegistry } = await import('./connectors')
  const { githubConnector } = await import('./connectors/github')
  const { mcpConnector } = await import('./connectors/mcp')
  connectorRegistry.register(githubConnector)
  connectorRegistry.register(mcpConnector)

  // Load initial config and wire up managers
  const config = configManager.loadConfig()
  setEnvPassthrough(config.defaults.envPassthrough)
  ptyManager.setAgentCommands(config.agentCommands)
  ptyManager.setRemoteHosts(config.remoteHosts ?? [])
  headlessManager.setAgentCommands(config.agentCommands)
  scheduler.syncSchedules(config.workflows ?? [])

  // Re-sync managers and broadcast to clients when config changes
  configManager.onConfigChanged((cfg) => {
    setEnvPassthrough(cfg.defaults.envPassthrough)
    ptyManager.setAgentCommands(cfg.agentCommands)
    ptyManager.setRemoteHosts(cfg.remoteHosts ?? [])
    headlessManager.setAgentCommands(cfg.agentCommands)
    scheduler.syncSchedules(cfg.workflows ?? [])
    clientRegistry.broadcast(IPC.CONFIG_CHANGED, cfg)
    // Auto-rebind when networkAccessEnabled changes, and re-read the names the
    // web client may be served from on the same transition.
    checkAndRebind().catch((err) => log.warn({ err }, '[server] rebind check failed'))
    void refreshTrustedOrigins()
  })

  // Set up Fastify + WebSocket
  const app = Fastify({ logger: false })
  await app.register(websocket)

  // Resolve this process's local credential and publish it for same-machine
  // tools, before any connection can be accepted.
  initBootstrapSecret(dataDir)

  app.get(
    '/ws',
    {
      websocket: true,
      // Refuse a foreign Origin at the upgrade rather than accepting the socket
      // and closing it. Browsers set this header and page script cannot forge it,
      // so this is what stops an arbitrary website opening a socket to a server
      // bound on loopback — which browsers permit, since WebSocket upgrades are
      // subject to neither CORS nor same-origin policy.
      preValidation: async (req, reply) => {
        if (!isAllowedUpgrade(req.headers.origin, req.headers.host)) {
          logRefusedUpgrade(req.headers.origin, req.headers.host)
          await reply.code(403).send({ error: 'Origin not allowed' })
        }
      }
    },
    (socket, req) => {
      handleConnection(socket, bearerFrom(req.headers.authorization), parseTopics(req.query))
      scheduler.deliverPendingConnectorInbox()
    }
  )

  app.get('/health', async () => ({ status: 'ok' }))

  /**
   * Pairing, the phone's half.
   *
   * HTTP rather than the socket, and polled rather than held open. A phone
   * that has not paired has no credential, and the socket admits exactly one
   * method before authenticating — widening that is the last thing worth doing
   * to reach a five minute flow. Worse, an unauthenticated socket is capped at
   * 64 with a ten second window, so holding one open for the length of a
   * pairing window turns that cap into a way to lock everyone else out.
   *
   * Neither route returns a token without a person having approved on the
   * machine being paired to. What they can be used for is burning a code the
   * owner is currently looking at, which the attempt cap bounds.
   */
  const requireJson = async (req: FastifyRequest, reply: FastifyReply): Promise<void> => {
    // A form post from a hostile page cannot set this content type without a
    // preflight, and no CORS headers are ever sent, so nothing cross-origin
    // reads the reply either.
    if (!req.headers['content-type']?.includes('application/json')) {
      await reply.code(415).send({ error: 'Expected application/json' })
    }
  }

  app.post('/api/pair/redeem', { preValidation: requireJson }, async (req, reply) => {
    const { code, deviceName } = (req.body ?? {}) as { code?: unknown; deviceName?: unknown }
    const result = redeemCode(code, deviceName, req.ip)
    if (!result.ok) return reply.code(400).send({ error: result.reason })

    const pending = pendingRequests().find((r) => r.requestId === result.requestId)
    // The desktop is told rather than asked to poll: the approval prompt has to
    // appear the moment the phone asks, not on the next refresh.
    if (pending) clientRegistry.broadcast(IPC.PAIRING_REQUESTED, pending)
    return { requestId: result.requestId }
  })

  app.post('/api/pair/poll', { preValidation: requireJson }, async (req) => {
    const { requestId } = (req.body ?? {}) as { requestId?: unknown }
    return pollRequest(requestId, os.hostname().replace(/\.local$/, ''))
  })

  // Serve task images via HTTP (used by web app instead of file:// protocol)
  app.get('/api/task-images/:taskId/:filename', async (req, reply) => {
    const { taskId, filename } = req.params as { taskId: string; filename: string }
    try {
      const filePath = resolveTaskImagePath(taskId, filename)
      if (!fs.existsSync(filePath)) {
        return reply.code(404).send({ error: 'Image not found' })
      }
      const ext = path.extname(filename).toLowerCase()
      // No `.svg` — see the note on ALLOWED_IMAGE_EXTENSIONS in task-images.ts.
      // Anything not listed is served as an opaque download rather than rendered.
      const mimeTypes: Record<string, string> = {
        '.png': 'image/png',
        '.jpg': 'image/jpeg',
        '.jpeg': 'image/jpeg',
        '.gif': 'image/gif',
        '.webp': 'image/webp',
        '.bmp': 'image/bmp'
      }
      reply.header('Content-Type', mimeTypes[ext] || 'application/octet-stream')
      // `private`: this is user content, and once the port is reachable over
      // plaintext HTTP a `public` response invites an intercepting proxy to store it.
      reply.header('Cache-Control', 'private, max-age=86400')
      reply.header('X-Content-Type-Options', 'nosniff')
      // Belt and braces against the SVG class above: with no script, object or
      // frame sources and a sandbox, a document served from here can do nothing
      // even if one ever reaches this route again.
      reply.header('Content-Security-Policy', "default-src 'none'; sandbox")
      reply.header('Cross-Origin-Resource-Policy', 'same-origin')
      const stream = fs.createReadStream(filePath)
      return reply.send(stream)
    } catch {
      return reply.code(400).send({ error: 'Invalid request' })
    }
  })

  // Serve web app static files at /app/ if the dist directory exists.
  // Dev: _dirname = packages/server/src → ../../web/dist
  // Prod: _dirname = Resources/server   → ../web/dist
  const webDistDir = fs.existsSync(path.resolve(_dirname, '../web/dist'))
    ? path.resolve(_dirname, '../web/dist')
    : path.resolve(_dirname, '../../web/dist')
  if (fs.existsSync(webDistDir)) {
    // The served bundle is the authenticated UI, and it is same-origin with the
    // device token the web client stores. Without this any page could frame it and
    // clickjack a session; the socket's Origin check does not help, because a
    // frame is a plain document load rather than an upgrade.
    app.addHook('onSend', async (req, reply) => {
      if (!req.url.startsWith('/app')) return
      reply.header('X-Frame-Options', 'DENY')
      reply.header('Content-Security-Policy', "frame-ancestors 'none'")
    })

    await app.register(fastifyStatic, {
      root: webDistDir,
      prefix: '/app/'
    })
    // SPA fallback: serve index.html for any /app/* route not matching a file
    app.setNotFoundHandler((req, reply) => {
      if (req.url.startsWith('/app')) {
        return reply.sendFile('index.html', webDistDir)
      }
      reply.code(404).send({ error: 'Not found' })
    })
    log.info(`[server] serving web app from ${webDistDir}`)
  }

  // Register all RPC methods
  registerAllMethods()
  scheduler.startInboxWorker()

  // Server shutdown method (callable from clients)
  registerMethod('server:shutdown', async () => {
    log.info('[server] shutdown requested via RPC')
    setTimeout(async () => {
      await shutdown()
    }, 100)
  })

  // Bind wide when remote access is enabled, else loopback. Tailscale used to be
  // required too, which made the tailnet the boundary; every connection is
  // authenticated now, so the credential is.
  const host = options.host ?? (config.defaults.networkAccessEnabled ? '0.0.0.0' : '127.0.0.1')
  if (host === '0.0.0.0') log.info('[server] remote access enabled, binding to 0.0.0.0')

  // Tailscale is now only a source of names the web client may be served from —
  // it no longer decides anything. Best-effort and non-blocking: a refused origin
  // by name still connects by IP literal, so a slow or absent Tailscale must not
  // hold up startup.
  void refreshTrustedOrigins()

  // Keep the same port across restarts. A browser keys localStorage by origin, so
  // an ephemeral port hands a paired device a new origin every launch and its
  // token goes with it — the user would experience that as Vorn forgetting them.
  // An explicit --port always wins; otherwise take the remembered one, falling
  // back if something else has claimed it meanwhile.
  const preferredPort = options.port ?? config.defaults.serverPort ?? 0
  try {
    await app.listen({ host, port: preferredPort })
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'EADDRINUSE' || preferredPort === 0) throw err
    log.warn(`[server] port ${preferredPort} is in use, taking another`)
    await app.listen({ host, port: 0 })
  }

  const address = app.server.address()
  const actualPort = typeof address === 'object' && address ? address.port : preferredPort

  // Remember it, so the next launch is the same origin. Written straight to the
  // database rather than through notifyChanged: a config broadcast here would
  // re-enter checkAndRebind during startup.
  if (!options.port && config.defaults.serverPort !== actualPort) {
    try {
      configManager.saveConfig({
        ...config,
        defaults: { ...config.defaults, serverPort: actualPort }
      })
    } catch (err) {
      log.warn({ err }, '[server] could not remember the port; it may change next launch')
    }
  }

  // Store port for RPC methods (e.g. tailscale:status needs it)
  setServerPort(actualPort)

  // Enable hot-rebind when network access / Tailscale state changes
  initRebind(app.server, host, actualPort)

  // The `{"port":N}` line that Electron's launcher waits for is written by the
  // direct-run block below, not here: it is a contract between that entry point
  // and its parent process, not a property of the server. The CLI has no parent
  // and prints something a person can read instead.

  // Write WS port to a well-known file so MCP and other tools can discover it.
  // Use JSON with PID so multiple instances don't clobber each other's port files.
  // Lives beside the database rather than always in ~/.vorn, so a server on its
  // own data dir advertises itself there instead of over the desktop's file.
  const wsPortFile = path.join(dataDir, 'ws-port')
  let ownsPortFile = true
  try {
    fs.mkdirSync(path.dirname(wsPortFile), { recursive: true })

    // Check if another live instance owns the port file
    try {
      const existing = JSON.parse(fs.readFileSync(wsPortFile, 'utf-8'))
      if (existing.pid && existing.pid !== process.pid) {
        try {
          process.kill(existing.pid, 0) // probe — throws if dead
          ownsPortFile = false
          log.info(
            { existingPid: existing.pid },
            '[server] another instance owns ws-port file, skipping write'
          )
        } catch {
          // dead PID — safe to overwrite
        }
      }
    } catch {
      // no file or invalid JSON — safe to write
    }

    if (ownsPortFile) {
      fs.writeFileSync(wsPortFile, JSON.stringify({ port: actualPort, pid: process.pid }), 'utf-8')
    }
  } catch (err) {
    log.warn({ err }, '[server] failed to write ws-port file (MCP discovery will not work)')
  }

  log.info(`[server] listening on ${host}:${actualPort}`)

  // Graceful shutdown
  const { hookServer } = await import('./hook-server')
  const { uninstallHooks } = await import('./hook-installer')
  const { uninstallAllCopilotHooks } = await import('./copilot-hook-installer')
  const { hookStatusMapper } = await import('./hook-status-mapper')
  const { sessionManager } = await import('./session-persistence')

  const shutdown = async () => {
    log.info('[server] shutting down...')
    // Stop the periodic timer first, then do one final synchronous save
    sessionManager.stopAutoSave()
    sessionManager.persistNow()
    hookServer.stop()
    clearLocalCredential()
    uninstallHooks()
    uninstallAllCopilotHooks()
    hookStatusMapper.clear()
    scheduler.stopAll()
    headlessManager.killAll()
    ptyManager.killAll()
    const { stopAllMcpClients } = await import('./connectors')
    await stopAllMcpClients()
    configManager.close()
    if (ownsPortFile) {
      try {
        const raw = JSON.parse(fs.readFileSync(wsPortFile, 'utf-8'))
        if (raw.pid === process.pid) fs.unlinkSync(wsPortFile)
      } catch {
        /* ignore */
      }
    }
    await app.close()
    process.exit(0)
  }

  process.on('SIGTERM', shutdown)
  process.on('SIGINT', shutdown)
  process.on('message', (msg) => {
    if (msg === 'shutdown') shutdown()
  })

  return { app, port: actualPort }
}

// Run directly
const isDirectRun =
  process.argv[1]?.endsWith('index.ts') ||
  process.argv[1]?.endsWith('index.js') ||
  process.argv[1]?.endsWith('index.cjs')
if (isDirectRun) {
  const { host, port, dataDir } = parseServerArgs(process.argv.slice(2))

  startServer({ port: port ?? 0, host, dataDir })
    .then(({ port: actualPort }) => {
      // This entry point is the one Electron forks, and its launcher blocks on
      // reading this line to learn where to connect. It belongs here rather than
      // inside startServer, which has no parent to answer to.
      process.stdout.write(JSON.stringify({ port: actualPort }) + '\n')
    })
    .catch((err) => {
      log.error({ err }, '[server] failed to start')
      const msg =
        '[server] failed to start: ' +
        (err instanceof Error ? err.stack || err.message : String(err))
      process.stderr.write(msg + '\n')
      process.exit(1)
    })
}
