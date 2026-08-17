import fs from 'node:fs'
import path from 'node:path'
import Fastify from 'fastify'

// CJS (prod bundle): __dirname is a global. ESM (dev/tsx): fall back to the
// directory of the entry script. Avoids import.meta.url which tsup emits as
// import_meta.url in the CJS bundle, crashing at runtime.
const _dirname = typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
import websocket from '@fastify/websocket'
import fastifyStatic from '@fastify/static'
import { handleConnection, registerMethod } from './ws-handler'
import { registerAllMethods, setServerPort } from './register-methods'
import { configManager } from './config-manager'
import { getDataDir } from './database'
import { parseServerArgs } from './server-args'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { scheduler } from './scheduler'
import { getTaskImagePath as resolveTaskImagePath } from './task-images'
import { getTailscaleStatus } from './tailscale'
import { initRebind, checkAndRebind } from './server-rebind'
import { setEnvPassthrough } from './process-utils'
import log from './logger'

export async function startServer(
  options: { host?: string; port?: number; dataDir?: string } = {}
) {
  // Initialize database + config. This resolves the data directory for the whole
  // process; everything else reads it back with getDataDir() rather than
  // deriving it again, so nothing can disagree about where the files are.
  configManager.init(options.dataDir)
  configManager.watchDb()
  const dataDir = getDataDir()

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
  const { clientRegistry } = await import('./broadcast')
  const { IPC } = await import('@vornrun/shared/types')
  configManager.onConfigChanged((cfg) => {
    setEnvPassthrough(cfg.defaults.envPassthrough)
    ptyManager.setAgentCommands(cfg.agentCommands)
    ptyManager.setRemoteHosts(cfg.remoteHosts ?? [])
    headlessManager.setAgentCommands(cfg.agentCommands)
    scheduler.syncSchedules(cfg.workflows ?? [])
    clientRegistry.broadcast(IPC.CONFIG_CHANGED, cfg)
    // Auto-rebind when networkAccessEnabled changes
    checkAndRebind().catch((err) => log.warn({ err }, '[server] rebind check failed'))
  })

  // Set up Fastify + WebSocket
  const app = Fastify({ logger: false })
  await app.register(websocket)

  app.get('/ws', { websocket: true }, (socket) => {
    handleConnection(socket)
    scheduler.deliverPendingConnectorInbox()
  })

  app.get('/health', async () => ({ status: 'ok' }))

  // Serve task images via HTTP (used by web app instead of file:// protocol)
  app.get('/api/task-images/:taskId/:filename', async (req, reply) => {
    const { taskId, filename } = req.params as { taskId: string; filename: string }
    try {
      const filePath = resolveTaskImagePath(taskId, filename)
      if (!fs.existsSync(filePath)) {
        return reply.code(404).send({ error: 'Image not found' })
      }
      const ext = path.extname(filename).toLowerCase()
      const mimeTypes: Record<string, string> = {
        '.png': 'image/png',
        '.jpg': 'image/jpeg',
        '.jpeg': 'image/jpeg',
        '.gif': 'image/gif',
        '.webp': 'image/webp',
        '.svg': 'image/svg+xml',
        '.bmp': 'image/bmp'
      }
      reply.header('Content-Type', mimeTypes[ext] || 'application/octet-stream')
      reply.header('Cache-Control', 'public, max-age=86400')
      reply.header('X-Content-Type-Options', 'nosniff')
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

  // Determine bind address: if networkAccessEnabled AND Tailscale is running,
  // bind to 0.0.0.0 so other devices on the tailnet can reach us.
  // Otherwise, localhost only.
  let host = options.host ?? '127.0.0.1'
  if (!options.host && config.defaults.networkAccessEnabled) {
    try {
      const tsStatus = await getTailscaleStatus()
      if (tsStatus.running && tsStatus.selfIP) {
        host = '0.0.0.0'
        log.info(
          `[server] remote access enabled, binding to 0.0.0.0 (tailscale IP: ${tsStatus.selfIP})`
        )
      }
    } catch (err) {
      log.warn({ err }, '[server] failed to check tailscale status, falling back to localhost')
    }
  }
  const port = options.port ?? 0 // 0 = OS-assigned

  await app.listen({ host, port })
  const address = app.server.address()
  const actualPort = typeof address === 'object' && address ? address.port : port

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
