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
import {
  extensionRenamedSession,
  reconcileImplicitConnections,
  registerAllMethods,
  setServerPort,
  sessionsToPersist
} from './register-methods'
import { registerWebhookRoute } from './webhook-trigger'
import { registerExtensionBridge, type ExtensionRouteDeps } from './extensions/bridge'
import { setExtensionBridgeOrigin, stopAllHosts } from './extensions/hosts'
import { startExtensionPageServer, stopExtensionPageServer } from './extensions/page-server'
import { stopAllFooters } from './extensions/footers'
import { abandonSelections } from './extensions/selection'
import { configManager } from './config-manager'
import { claimPublishedFiles, writePortFile, removePortFile } from './published-files'
import { openLocalEndpoint, type LocalEndpoint } from './local-endpoint'
import { handOver, type HandoffHost } from './handoff/donor'
import { receiveHandoff, announceServing, type AdoptedPane } from './handoff/heir'
import { releaseListener, retakeListener } from './server-rebind'
import { beginDraining, isDraining, watchEndpoint } from './draining'
import {
  initBootstrapSecret,
  publishLocalCredential,
  clearLocalCredential,
  bearerFrom
} from './ws-auth'
import { getDataDir, dbCountActiveConnectorInboxLeases } from './database'
import { parseServerArgs, resolveServerPort, shouldRememberPort } from './server-args'
import {
  DEFAULT_SERVER_PORT,
  EXIT_ENDPOINT_TAKEN,
  SERVER_LOG_FILENAME
} from '@vornrun/shared/protocol'
import { ptyManager } from './pty-manager'
import { configureHistory, flushHistory, checkpointAll } from './history/writer'
import { recoverHistory } from './history/recovery'
import { seedRestored, markRecovered, verifyRestored, consumeRestored } from './restored-sessions'
import { getGitBranchAsync, getGitHeadAsync } from './git-utils'
import { sessionManager } from './session-persistence'
import { headlessManager } from './headless-manager'
import { scheduler } from './scheduler'
import { resumeRunsAfterStart } from './workflows/resume'
import { getTaskImagePath as resolveTaskImagePath } from './task-images'
import { redeemCode, pollRequest, pendingRequests } from './pairing'
import { getTailscaleStatus } from './tailscale'
import { initRebind, checkAndRebind, getCurrentHost } from './server-rebind'
import { isAllowedUpgrade, logRefusedUpgrade, setTrustedOriginHosts } from './ws-origin'
import {
  primeShellEnv,
  shellEnvSettled,
  setEnvPassthrough,
  setLaunchDataDir
} from './process-utils'
import log from './logger'
import { appFrameAncestors } from './extensions/frame-ancestors'

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
 * How long a shutdown may take before this process leaves regardless.
 *
 * Generous, because the work it waits on is real -- persisting sessions, killing
 * terminals, stopping connector subprocesses -- and cutting it short loses more
 * than it saves. It exists only for the case where one of those never returns.
 */
const SHUTDOWN_DEADLINE_MS = 30_000

/**
 * How long everything must stay empty before the server stops.
 *
 * Not a setting. A number to tune is a knob nobody wants, and the honest range
 * is narrow: with nothing running there is no work to lose by exiting, so the
 * window is only there to avoid churning a process for somebody who is coming
 * straight back. The environment variable exists for tests, which cannot wait
 * half an hour to watch a process leave.
 */
function resolveIdleWindowMs(): number {
  const raw = process.env.VORN_IDLE_TIMEOUT_MS
  const parsed = raw ? Number(raw) : NaN
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_IDLE_WINDOW_MS
}

/**
 * `dev` or `packaged`, preferring what the launcher told us.
 *
 * The fallback reads the entry point rather than NODE_ENV: NODE_ENV is set to
 * 'production' by the packaged launcher AND left at whatever the shell had for a
 * CLI run, so it answers a different question than the one being asked. The entry
 * extension is the fact itself — `.ts` only runs under tsx from a checkout.
 */
function resolveBuildChannel(): 'dev' | 'packaged' {
  const declared = process.env.VORN_BUILD_CHANNEL
  if (declared === 'dev' || declared === 'packaged') return declared
  return process.argv[1]?.endsWith('.ts') ? 'dev' : 'packaged'
}

/**
 * Origins that may frame a pane's page, filled in once this server has a port.
 *
 * The pages are on their own origin, so the app's is not implied; naming it here
 * is what lets a window frame one at all.
 */
let extensionFrameAncestors: string[] = []

const extensionRouteDeps: ExtensionRouteDeps = {
  frameAncestors: () => extensionFrameAncestors,
  sessionRenamed: extensionRenamedSession
}

export async function startServer(
  options: {
    host?: string
    port?: number
    dataDir?: string
    idleShutdown?: boolean
    /** Origins beyond this server's own that may frame a pane, such as the desktop's. */
    extensionFrameAncestors?: string[]
    /** Terminals inherited from the server this one replaces, already taken and paused. */
    adopted?: AdoptedPane[]
  } = {}
) {
  const bootStarted = Date.now()
  // First, so the shell answers while the database opens and the modules load.
  void primeShellEnv()
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
  // Before anything listens, so there is no window where a session is created
  // and then never recorded. Nothing is written until a terminal exists, and a
  // server that loses the endpoint claim exits without ever having one.
  configureHistory(dataDir)

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

  // Register built-in connectors. Only the two that are a transport rather than
  // a product: everything that speaks to a named service is a pack.
  const { connectorRegistry } = await import('./connectors')
  const { httpConnector } = await import('./connectors/http')
  const { mcpConnector } = await import('./connectors/mcp')
  connectorRegistry.register(httpConnector)
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

  // Who owns this data directory's published names, decided once and used by
  // every publisher below.
  //
  // Taken here rather than beside the port-file write, which is where it used to
  // live, because the credential is published before the listen and the port
  // after it — two publishers asking the same question at two different moments
  // is how they came to disagree. `~/.vorn` is shared by a packaged Vorn and a
  // `yarn dev` server on purpose, so a fixed name in it needs an owner or the
  // last writer silently wins.
  const ownsPublished = claimPublishedFiles(dataDir)

  // Resolve this process's local credential, before any connection can be
  // accepted. Publishing it is a separate step, after the endpoint is claimed:
  // the secret has to exist to authenticate anyone, but the file announces this
  // server as the one this machine uses, which is not true until it has won.
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
        { transport: 'tcp', address: req.socket.remoteAddress }
      )
      scheduler.deliverPendingConnectorInbox()
    }
  )

  app.get('/health', async () => ({ status: 'ok' }))

  registerWebhookRoute(app, () => scheduler.deliverPendingConnectorInbox())

  // An extension's own bridge, which its child process reaches with the token it
  // was started with. The pages its panes are drawn from are served on their own
  // origin instead, so a page shares neither storage nor a socket with the app.
  registerExtensionBridge(app, extensionRouteDeps)

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

  // Before `registerAllMethods()`, and that ordering is the point rather than
  // tidiness. Registering wires `startAutoSave`, and `startInboxWorker()` on the
  // next line can launch a workflow session -- so a save can fire from here, and
  // a save is a whole-table replace. Seeding after it would mean the records
  // this is holding had already been erased by the thing it exists to survive.
  // Uptime counts through sleep, so a laptop closed overnight is not a reboot.
  const bootTime = Date.now() - os.uptime() * 1000
  const carriedOver = seedRestored(sessionManager.readPreviousSessions(), Date.now(), bootTime)

  const adopted = options.adopted ?? []
  // Handed-over sessions are live, but the previous server wrote them down on its
  // way out, so they are also sitting in the list above being offered as resumable.
  for (const pane of adopted) consumeRestored(pane.session.id)

  // Register all RPC methods
  registerAllMethods()

  // Connects the rung-none packs installed before installing meant connecting.
  reconcileImplicitConnections()
  scheduler.startInboxWorker()
  // After the methods, because picking a run back up uses them.
  void resumeRunsAfterStart()

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
  // The address the extension children are given, known only once a port is won.
  setExtensionBridgeOrigin(`http://127.0.0.1:${actualPort}`)

  // Pane pages, on a port of their own. The app frames them, so the app's origins
  // are the only ones allowed to; a page that fails to start costs its panes, not
  // the server.
  const appOrigins = [`http://127.0.0.1:${actualPort}`, `http://localhost:${actualPort}`]
  extensionFrameAncestors = [
    ...appOrigins,
    ...appFrameAncestors(process.env.VORN_APP_ORIGINS),
    ...(options.extensionFrameAncestors ?? [])
  ]
  try {
    await startExtensionPageServer(extensionRouteDeps)
  } catch (err) {
    log.warn({ err }, '[extensions] pane pages have no origin; panes will not open')
  }

  // Enable hot-rebind when network access / Tailscale state changes
  initRebind(app.server, host, actualPort)

  // The `{"port":N}` line that Electron's launcher waits for is written by the
  // direct-run block below, not here: it is a contract between that entry point
  // and its parent process, not a property of the server. The CLI has no parent
  // and prints something a person can read instead.

  // The canonical local endpoint. A name that can be owned, unlike the port
  // above -- the desktop probes and adopts through it, and holding it is what
  // makes this server the one this machine answers with.
  //
  // Opened after the listen rather than before: a server that cannot claim the
  // name still serves whatever is already attached to it, and losing the claim
  // must never be the reason a startup fails.
  const claimed = await openLocalEndpoint(dataDir, () => scheduler.deliverPendingConnectorInbox())

  // Arriving second is not a failure, and it is not something to carry on
  // through. This process would be a second server on one database, and
  // `saveSessions` is a whole-table replace -- two of them erase each other's
  // sessions, which is the hazard adoption exists to prevent and the reason a
  // refused launch does not spawn a rival. So it stands down before publishing
  // anything, on an exit code that says which of the three things happened, and
  // the app that started it adopts the incumbent instead of relaunching this.
  if (claimed.kind === 'lost') {
    log.warn({ because: claimed.because }, '[server] this machine already has a server')
    await app.close()
    process.exit(EXIT_ENDPOINT_TAKEN)
  }
  // `let`, because a handoff gives it up and an abandoned handoff takes it back.
  let endpoint = claimed.kind === 'held' ? claimed.endpoint : null
  // Asked at session creation rather than only on the idle tick: that timer runs
  // at a quarter of the window and is switched off entirely for `vorn-server
  // serve`, so a check that lived only there would be late or absent exactly
  // where it matters.
  if (endpoint) watchEndpoint(() => endpoint?.holds() ?? false)

  // After the claim, so a server that arrives second exits above rather than
  // replaying every terminal first; awaited before the port file, so no client
  // can be told there is no history. Adopted sessions are rebuilt from the
  // previous server's checkpoint, and a session absent from the list has its
  // history swept. Null sweeps nothing.
  const recoverable =
    carriedOver === null
      ? null
      : [
          ...carriedOver,
          ...adopted
            .filter((pane) => !carriedOver.some((s) => s.id === pane.session.id))
            .map((pane) => pane.session)
        ]
  if (carriedOver === null && adopted.length) {
    log.warn(
      '[handoff] the session list could not be read; adopted terminals start without scrollback'
    )
  }
  const recovering = recoverHistory(dataDir, recoverable)
  // git is worth a short wait for the shell's PATH; a slow shell is not worth the boot.
  const verifying = shellEnvSettled(1000).then(() =>
    verifyRestored({
      isDirectory: (at) => {
        try {
          return fs.statSync(at).isDirectory()
        } catch {
          return false
        }
      },
      branch: getGitBranchAsync,
      head: getGitHeadAsync
    })
  )

  /** Registered here because every closure needs the endpoint, port and file ownership. */
  const handoffHost: HandoffHost = {
    dataDir,
    describePanes: () => ptyManager.describeForHandoff(),
    pauseAll: () => ptyManager.pauseAllForHandoff(),
    resumeAll: () => {
      ptyManager.resumeAllForHandoff()
      // A handoff that did not happen must not leave this server never persisting again.
      sessionManager.startAutoSave(sessionsToPersist)
    },
    quiesce: async () => {
      // What a shutdown does, minus the sealing and the killing. Written down
      // before the auto-save is stopped, because stopping it drops the source
      // `persistNow` reads -- the other order saves nothing at all.
      sessionManager.persistNow()
      sessionManager.stopAutoSave()
      ptyManager.flushPendingOutput()
      await checkpointAll()
    },
    openLogFd: () => fs.openSync(path.join(dataDir, SERVER_LOG_FILENAME), 'a'),
    release: async () => {
      // The order is the commit: name, then port file, then listener.
      //
      // `relinquish`, not `close`: the request being served arrived on this
      // endpoint and its reply has to travel back out over it.
      //
      // The draining watch is pointed away first, or giving the name up
      // deliberately would latch it irreversibly on a server that may roll back.
      watchEndpoint(() => true)
      endpoint?.relinquish()
      removePortFile(dataDir, ownsPublished)
      await releaseListener()
    },
    reclaim: async () => {
      const listening = await retakeListener()
      // A fresh listener: an anonymous inode cannot be linked back to a path. The
      // old one stays open, because this request's socket is still on it.
      const again = await openLocalEndpoint(dataDir, () => scheduler.deliverPendingConnectorInbox())
      if (again.kind === 'held') endpoint = again.endpoint
      // Restored on both paths. `release` pointed the watch at `true` so giving
      // the name up deliberately would not latch draining; left there after a
      // reclaim that failed, this server would never notice it has no endpoint
      // and would go on creating sessions nothing can reach.
      watchEndpoint(() => endpoint?.holds() ?? false)
      if (listening) writePortFile(dataDir, actualPort, ownsPublished)
      return listening && again.kind === 'held'
    },
    // Not `shutdown()`: that kills every PTY, which is the one thing a handoff must not do.
    exit: () => process.exit(0)
  }

  registerMethod('server:handoff', (params) => handOver(params, handoffHost))

  markRecovered((await recovering).recovered)
  await verifying

  // After recovery, whose rebuilt screens `createScreen` would otherwise clear, and
  // before the port file, so no client can find this server and be told it is empty.
  ptyManager.adoptPanes(adopted)

  // Published together, after the claim, because they are one announcement: the
  // port says where, the credential says how, and a reader that finds one
  // without the other cannot reach anything.
  publishLocalCredential(ownsPublished)
  writePortFile(dataDir, actualPort, ownsPublished)

  log.info(`[server] listening on ${host}:${actualPort} (ready in ${Date.now() - bootStarted}ms)`)

  // Graceful shutdown
  const { hookServer } = await import('./hook-server')
  const { uninstallHooks } = await import('./hook-installer')
  const { uninstallAllCopilotHooks } = await import('./copilot-hook-installer')
  const { hookStatusMapper } = await import('./hook-status-mapper')

  // Two failures, and neither is retried -- by the time either is visible this
  // has already cleared the credential and removed the port file, so a second
  // attempt would be running `killAll()` and `app.close()` again over a server
  // no app can discover anyway. The worst outcome is not a failed shutdown, it
  // is a live process still holding the port with nothing able to reach it.
  //
  // So a throw takes the hard exit at the call site, and a hang -- which is
  // reachable, `stopAllMcpClients()` awaits child processes -- takes the
  // deadline armed here. Re-entry is refused because SIGTERM can arrive while
  // one of those is already in flight. Unref'd: a backstop, not a reason to
  // stay alive.
  let shuttingDown = false
  const shutdown = async () => {
    if (shuttingDown) return
    shuttingDown = true
    setTimeout(() => {
      log.error('[server] shutdown did not finish; exiting anyway')
      process.exit(1)
    }, SHUTDOWN_DEADLINE_MS).unref?.()
    // The final save first: `stopAutoSave` drops the session source, so the
    // other order made this write nothing on every shutdown.
    sessionManager.persistNow()
    sessionManager.stopAutoSave()
    // The last few milliseconds of output, then every terminal's screen, before
    // anything kills a PTY. After `killAll()` the buffers this reads have been
    // emptied; before `persistNow()` the sessions these belong to are not yet
    // saved. Awaited because serializing a screen is asynchronous by necessity,
    // and bounded from the inside -- it is on the same path as the unref'd
    // `SHUTDOWN_DEADLINE_MS` and must not be able to spend all of it.
    ptyManager.flushPendingOutput()
    await flushHistory()
    hookServer.stop()
    clearLocalCredential()
    uninstallHooks()
    uninstallAllCopilotHooks()
    hookStatusMapper.clear()
    scheduler.stopAll()
    headlessManager.killAll()
    ptyManager.killAll()
    stopAllFooters()
    abandonSelections()
    await stopExtensionPageServer()
    await stopAllHosts()
    const { stopAllMcpClients } = await import('./connectors')
    await stopAllMcpClients()
    configManager.close()
    removePortFile(dataDir, ownsPublished)
    // Never removes the canonical entry: this listener bound a scratch name that
    // no longer exists, so libuv's unlink at close has nothing to find. A dead
    // socket file left behind is the next publisher's to replace in one rename.
    await endpoint?.close()
    await app.close()
    process.exit(0)
  }

  /**
   * Every way this process is asked to stop, so there is one of them.
   *
   * A rejected `shutdown()` reaches here rather than becoming an unhandled
   * rejection. By the time one is visible the credential is cleared and the port
   * file is gone, so there is nothing to salvage and nothing to retry -- what is
   * left is a live process holding a port no app can discover, which is the state
   * this whole feature exists to prevent. Exiting hard is the floor. A shutdown
   * that hangs rather than rejects is caught by the deadline armed inside it.
   */
  const stopFor = (reason: string): void => {
    log.info({ reason }, '[server] shutting down')
    void shutdown().catch((err) => {
      log.error({ err, reason }, '[server] shutdown failed; exiting anyway')
      process.exit(1)
    })
  }

  /**
   * Whether this server is winding down, checking first whether it still holds
   * the endpoint it claimed.
   *
   * One direction only: once lost, a name is not given back. A server that never
   * held one is not draining -- it is running TCP-only, which is a downgrade
   * rather than a loss, and refusing its sessions would leave the machine with
   * nothing that works.
   */
  let saidSo = false
  const noticeLostEndpoint = (held: LocalEndpoint | null): boolean => {
    // The endpoint is asked before `isDraining()`, not after. That call flips the
    // flag itself now -- session creation consults the same check, so it has to --
    // and asking it first meant this function's own transition never ran and the
    // warning below was dead. A server quietly refusing every new session with
    // nothing in the log saying why is a bad half-hour for whoever hits it.
    const lost = held !== null && !held.holds()
    if (lost && !saidSo) {
      saidSo = true
      log.warn('[endpoint] this server no longer holds the endpoint; finishing what it has')
      beginDraining()
    }
    return isDraining()
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
      servesOthers: getCurrentHost() === '0.0.0.0',
      // Looked at rather than waited for. Losing the endpoint is something that
      // happens *to* this process -- another server that found it unreachable is
      // entitled to take the name and has no way to say so. One lstat per tick
      // is the cheapest place to notice, and this timer already runs.
      draining: noticeLostEndpoint(endpoint)
    }),
    { windowMs: resolveIdleWindowMs(), schedulesHoldOpen: true },
    () => stopFor('nothing left to do')
  )
  // Off entirely for a hand-run `vorn-server serve`: that process is the thing
  // being run, and nothing would bring it back. Being bound wide is handled in
  // the snapshot instead, because it can change while this runs.
  if (options.idleShutdown === false) {
    log.info('[server] idle shutdown off: this server was started to be run')
  } else {
    idleWatch.start()
  }

  process.on('SIGTERM', () => stopFor('SIGTERM'))
  process.on('SIGINT', () => stopFor('SIGINT'))
  // Why: this process outlives the app that started it, so a hangup on the
  // terminal or the departure of a parent must not take the sessions with it.
  // SIGTERM stays honoured — that is a request to stop, not an accident.
  process.on('SIGHUP', () => {
    log.info('[server] ignoring SIGHUP; sessions keep running')
  })
  process.on('message', (msg) => {
    if (msg === 'shutdown') stopFor('the app asked')
  })

  return { app, port: actualPort, idleWatch }
}

// Run directly
const isDirectRun =
  process.argv[1]?.endsWith('index.ts') ||
  process.argv[1]?.endsWith('index.js') ||
  process.argv[1]?.endsWith('index.cjs')
if (isDirectRun) {
  const { host, port, dataDir, adoptHandoff } = parseServerArgs(process.argv.slice(2))

  /**
   * Before `startServer`, and that ordering is the transaction: the cheap certain
   * work while failing is still free, the fallible work after the commit.
   */
  const adopting = adoptHandoff ? receiveHandoff(adoptHandoff) : Promise.resolve([])

  adopting
    .then((adopted) => {
      if (adopted === null) {
        log.error('[handoff] could not take the terminals; leaving the previous server with them')
        process.exit(1)
      }
      return startServer({ port, host, dataDir, adopted })
    })
    .then(({ port: actualPort }) => {
      // The last moment the handoff could have been abandoned.
      if (adoptHandoff) announceServing()
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
