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
import {
  handleConnection,
  registerMethod,
  setServerIdentity,
  setLiveSessionCount
} from './ws-handler'
import { IdleWatch, DEFAULT_IDLE_WINDOW_MS } from './idle'
import { browserBridge } from './browser-bridge'
import { parseTopics, clientRegistry } from './broadcast'
import { IPC } from '@vornrun/shared/types'
import { registerAllMethods, setServerPort } from './register-methods'
import { configManager } from './config-manager'
import { initBootstrapSecret, clearLocalCredential, bearerFrom } from './ws-auth'
import { getDataDir, dbCountActiveConnectorInboxLeases } from './database'
import { parseServerArgs, resolveServerPort, shouldRememberPort } from './server-args'
import { DEFAULT_SERVER_PORT, WS_PORT_FILENAME } from '@vornrun/shared/protocol'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { scheduler } from './scheduler'
import { getTaskImagePath as resolveTaskImagePath } from './task-images'
import { redeemCode, pollRequest, pendingRequests } from './pairing'
import { getTailscaleStatus } from './tailscale'
import { initRebind, checkAndRebind, getCurrentHost } from './server-rebind'
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

/**
 * `dev` or `packaged`, preferring what the launcher told us.
 *
 * The fallback reads the entry point rather than NODE_ENV: NODE_ENV is set to
 * 'production' by the packaged launcher AND left at whatever the shell had for a
 * CLI run, so it answers a different question than the one being asked. The entry
 * extension is the fact itself — `.ts` only runs under tsx from a checkout.
 */
/**
 * How long everything must stay empty before the server stops.
 *
 * Not a setting. A number to tune is a knob nobody wants, and the honest range
 * is narrow: with nothing running there is no work to lose by exiting, so the
 * window is only there to avoid churning a process for somebody who is coming
 * straight back. The environment variable exists for tests, which cannot wait
 * half an hour to watch a process leave.
 */
/**
 * How long a shutdown may take before this process leaves regardless.
 *
 * Generous, because the work it waits on is real -- persisting sessions, killing
 * terminals, stopping connector subprocesses -- and cutting it short loses more
 * than it saves. It exists only for the case where one of those never returns.
 */
const SHUTDOWN_DEADLINE_MS = 30_000

function resolveIdleWindowMs(): number {
  const raw = process.env.VORN_IDLE_TIMEOUT_MS
  const parsed = raw ? Number(raw) : NaN
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_IDLE_WINDOW_MS
}

function resolveBuildChannel(): 'dev' | 'packaged' {
  const declared = process.env.VORN_BUILD_CHANNEL
  if (declared === 'dev' || declared === 'packaged') return declared
  return process.argv[1]?.endsWith('.ts') ? 'dev' : 'packaged'
}

export async function startServer(
  options: { host?: string; port?: number; dataDir?: string; idleShutdown?: boolean } = {}
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

  // Who this server is, so a desktop can decide whether to adopt it instead of
  // starting a second one on the same data directory. The channel is passed by
  // the launcher when there is one; a server started from the CLI infers it from
  // its own entry point, which is TypeScript in a checkout and bundled CJS in a
  // packaged app. Getting this wrong is not cosmetic: dev and packaged builds
  // deliberately share ~/.vorn, so a wrong answer lets one adopt the other's.
  // What a launcher deciding whether to adopt this server gets to hear before it
  // has authenticated — see `ServerIdentity.sessions`.
  setLiveSessionCount(() => ptyManager.livePtyCount())
  setServerIdentity({
    // The launcher passes the app's version; a CLI server has none to report and
    // says so rather than omitting the field, since every field on this frame is
    // required and a reader that has it should be done checking.
    appVersion: process.env.VORN_APP_VERSION ?? 'unknown',
    dataDir,
    pid: process.pid,
    buildChannel: resolveBuildChannel()
  })

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
      handleConnection(
        socket,
        bearerFrom(req.headers.authorization),
        parseTopics(req.query),
        // Decides whether the greeting carries this server's identity. Only a
        // desktop on this machine has any use for it, and only loopback can be
        // trusted not to be a stranger on the tailnet.
        req.socket.remoteAddress
      )
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
    const result = pollRequest(requestId, os.hostname().replace(/\.local$/, ''))
    // The token comes into existence here rather than at approval, so this is
    // the only moment a device list can be told it has something new to show.
    if (result.status === 'approved' && typeof requestId === 'string') {
      clientRegistry.broadcast(IPC.PAIRING_COLLECTED, { requestId })
    }
    return result
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
  // a moving port hands a paired device a new origin every launch and its token
  // goes with it — the user would experience that as Vorn forgetting them. An
  // explicit --port always wins; otherwise take the remembered one, or the
  // default on a first run, falling back only if something else holds it.
  const wantedPort = resolveServerPort({
    explicit: options.port,
    remembered: config.defaults.serverPort,
    fallback: DEFAULT_SERVER_PORT
  })
  let fellBack = false
  try {
    await app.listen({ host, port: wantedPort })
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'EADDRINUSE' || wantedPort === 0) throw err
    fellBack = true
    await app.listen({ host, port: 0 })
  }

  const address = app.server.address()
  const actualPort = typeof address === 'object' && address ? address.port : wantedPort

  if (fellBack) {
    // Said in full rather than as "taking another", because this is the line that
    // explains why a paired phone or a signed-in browser has just stopped
    // working: the origin moved. Two Vorn instances on one data directory is the
    // ordinary cause, a dev server beside the packaged app.
    log.warn(
      `[server] port ${wantedPort} is held by something else, so this server took ` +
        `${actualPort} instead. Anything paired to ${wantedPort} must be pointed at the new port.`
    )
  }

  // Remember it, so the next launch is the same origin. Written straight to the
  // database rather than through notifyChanged: a config broadcast here would
  // re-enter checkAndRebind during startup.
  //
  // `shouldRememberPort` carries the rule, including why a fallback port is
  // written on a first run and withheld from an install that already has one.
  const remember = shouldRememberPort({
    explicit: options.port,
    remembered: config.defaults.serverPort,
    fellBack
  })
  if (remember && config.defaults.serverPort !== actualPort) {
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
  const wsPortFile = path.join(dataDir, WS_PORT_FILENAME)
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

  // Two failures, and they need different answers. A shutdown that *throws* is
  // retried by the idle watch, which is why that watch keeps ticking. A shutdown
  // that *hangs* -- `stopAllMcpClients()` awaits child processes that can -- must
  // not be retried, because `killAll()` and `app.close()` would run a second
  // time; but leaving it hung is the worst state of all, a live process holding
  // the port with its credential already cleared and its port file already gone.
  // So re-entry is refused and a deadline is armed instead. Unref'd: it is a
  // backstop, not a reason to stay alive.
  let shuttingDown = false
  const shutdown = async () => {
    if (shuttingDown) return
    shuttingDown = true
    setTimeout(() => {
      log.error('[server] shutdown did not finish; exiting anyway')
      process.exit(1)
    }, SHUTDOWN_DEADLINE_MS).unref?.()
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

  // The server outlives the app now, so something has to decide when it is done.
  // Nothing here waits for the event loop to drain: the scheduler's inbox
  // interval is not unref'd, so this process would sit empty for ever.
  const idleWatch = new IdleWatch(
    () => ({
      sessions: ptyManager.livePtyCount(),
      // Filtered to running: `headless-manager` keeps exited entries for thirty
      // seconds, and a finished agent is not a reason to stay up.
      headless: headlessManager.getActiveSessions().filter((h) => h.status === 'running').length,
      msSinceClientActivity: clientRegistry.msSinceActivity(),
      msSinceHookActivity: hookServer.msSinceHookActivity(),
      bridgeAttached: browserBridge.isConnected,
      pendingPermissions: hookServer.getPendingPermissions().length,
      pendingPairings: pendingRequests().length,
      connectorLeases: dbCountActiveConnectorInboxLeases(new Date().toISOString()),
      enabledSchedules: scheduler.serverSideScheduleCount(),
      servesOthers: getCurrentHost() === '0.0.0.0'
    }),
    { windowMs: resolveIdleWindowMs(), schedulesHoldOpen: true },
    () => {
      log.info('[server] nothing left to do; shutting down')
      // A shutdown that throws has already cleared the credential and removed
      // the port file, so giving up here would leave a process bound to a port
      // no app can use or discover -- the exact state this feature exists to
      // prevent. Exiting hard is the floor. A shutdown that hangs instead is
      // caught by the deadline armed inside it.
      void shutdown().catch((err) => {
        log.error({ err }, '[server] idle shutdown failed; exiting anyway')
        process.exit(1)
      })
    }
  )
  // Off entirely for a hand-run `vorn-server serve`: that process is the thing
  // being run, and nothing would bring it back. Being bound wide is handled in
  // the snapshot instead, because it can change while this runs.
  if (options.idleShutdown === false) {
    log.info('[server] idle shutdown off: this server was started to be run')
  } else {
    idleWatch.start()
  }

  process.on('SIGTERM', shutdown)
  process.on('SIGINT', shutdown)
  // Why: this process outlives the app that started it, so a hangup on the
  // terminal or the departure of a parent must not take the sessions with it.
  // SIGTERM stays honoured — that is a request to stop, not an accident.
  process.on('SIGHUP', () => {
    log.info('[server] ignoring SIGHUP; sessions keep running')
  })
  process.on('message', (msg) => {
    if (msg === 'shutdown') shutdown()
  })

  return { app, port: actualPort, idleWatch }
}

// Run directly
const isDirectRun =
  process.argv[1]?.endsWith('index.ts') ||
  process.argv[1]?.endsWith('index.js') ||
  process.argv[1]?.endsWith('index.cjs')
if (isDirectRun) {
  const { host, port, dataDir } = parseServerArgs(process.argv.slice(2))

  startServer({ port, host, dataDir })
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
