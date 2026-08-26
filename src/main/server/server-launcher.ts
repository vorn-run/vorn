import { spawn, type ChildProcess } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import path from 'node:path'
import { createInterface } from 'node:readline'
import { app, utilityProcess, type UtilityProcess } from 'electron'
import log from '../logger'
import { BOOTSTRAP_ENV_VAR, SERVER_PORT_ENV_VAR } from '@vornrun/shared/protocol'
import { ServerBridge } from './server-bridge'
import { readHostSettings } from './host-store'
import { attemptsAfterExit, decideRelaunch } from './server-relaunch'

let serverProcess: ChildProcess | UtilityProcess | null = null
let bridge: ServerBridge | null = null

/**
 * The credential the server is spawned with, kept for as long as the app runs.
 *
 * It was a local in `launchServer`, which was fine while a server was started
 * exactly once. A relaunch has to spawn the replacement with the same one: the
 * bridge holds it and re-sends it on every reconnect, so a fresh token would
 * leave it authenticating against a server that had never heard of it.
 */
let bootstrapToken: string | null = null

/** Reset whenever a server has run long enough to look healthy. */
let relaunchAttempts = 0
/** When the current server was spawned, to tell a crash loop from bad luck. */
let lastSpawnAt = 0
/** Set by `stopServer`, so an exit we asked for is not treated as a failure. */
let stoppingDeliberately = false
/** A relaunch waiting to happen, so shutting down can call it off. */
let relaunchTimer: NodeJS.Timeout | null = null

/**
 * Connect to a server running on another machine.
 *
 * No spawn: two servers means two databases, and the local one would silently
 * shadow the host the user asked for. The credential also changes with the mode —
 * the per-launch bootstrap secret only authenticates a server this app started, so
 * a remote host needs a device token, which is why one has to exist before this
 * can work.
 */
async function connectToHost(url: string, token: string): Promise<ServerBridge> {
  log.info(`[launcher] connecting to host ${url}`)
  bridge = new ServerBridge(url, token)
  bridge.connect()

  try {
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error(`Could not reach ${url}`)),
        HOST_CONNECT_TIMEOUT_MS
      )
      bridge!.once('connected', () => {
        clearTimeout(timeout)
        resolve()
      })
    })
  } catch (err) {
    // Give up on the socket as well as the wait. The bridge reconnects every two
    // seconds on its own and nothing here would ever act on a late success, so
    // leaving it running means a timer and a listener churning behind the connect
    // window for as long as the app is open, against a host that may never answer.
    bridge?.close()
    bridge = null
    throw err
  }

  return bridge
}

/** Longer than the local wait: a remote host is a network away, not a fork away. */
const HOST_CONNECT_TIMEOUT_MS = 15_000

/**
 * Spawns the @vornrun/server process and returns a connected ServerBridge.
 *
 * The server writes `{"port": N}` to stdout on startup.
 * We read the port, then connect a WebSocket bridge.
 *
 * In dev mode: uses `npx tsx` via child_process.spawn (TypeScript execution).
 * In production: uses Electron's utilityProcess.fork() to run the bundled
 * server script as a Node.js process WITHOUT launching another Electron window.
 *
 * NOTE: Previously this used `spawn(process.execPath, ...)` in production,
 * which caused an infinite app spawn loop because process.execPath is the
 * Electron binary — spawning it launches another full Electron app instance.
 */
/**
 * `VORN_SERVER_PORT`, if it is a port.
 *
 * Checked here rather than forwarded blind. `parseServerArgs` does reject a
 * non-numeric `--port`, but it runs outside the server's `.catch`, so a typo
 * would take the process down with a stack trace while this launcher sat waiting
 * for a port line that was never coming — a hang, thirty seconds from the
 * mistake, describing none of it.
 *
 * Loud and ignored rather than fatal. `yarn dev` should still start; what it must
 * not do is come up quietly on a different port than the one asked for, which is
 * the confusion this whole mechanism exists to end.
 */
function readDevPort(): string | undefined {
  const raw = process.env[SERVER_PORT_ENV_VAR]
  if (!raw) return undefined

  const port = Number(raw)
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    log.warn(
      `[launcher] ${SERVER_PORT_ENV_VAR}="${raw}" is not a port, so it was ignored. ` +
        'The server will take its usual one.'
    )
    return undefined
  }

  log.info(`[launcher] ${SERVER_PORT_ENV_VAR}=${port}, asking for that port`)
  return String(port)
}

/**
 * Start a server process and wait for it to say where it is listening.
 *
 * Extracted from `launchServer` so a relaunch can run it again. Everything the
 * bridge needs afterwards -- the port -- comes back from here; everything it
 * already has -- the credential -- is module state now, because a replacement
 * server must be given the same one.
 */
async function spawnServer(): Promise<number> {
  lastSpawnAt = Date.now()
  const serverEntryPoint = resolveServerEntry()
  log.info(`[launcher] starting server: ${serverEntryPoint}`)

  // A per-run credential for the servers we spawn. Generated here rather than
  // persisted: nothing to leak at rest, and it dies with the app. The name is
  // stripped unconditionally by `filterEnv`, so it never reaches a PTY, headless
  // agent or script node. Reused across a relaunch -- see the declaration.
  bootstrapToken ??= randomBytes(32).toString('base64url')

  // Deliberately does NOT pass --data-dir. Vorn's data directory is ~/.vorn —
  // that is where the database, ws-port file and scheduler locks have always
  // lived, and where `packages/mcp` looks for all three. This used to pass
  // Electron's userData, which the server then ignored for everything except
  // task images, so it was inert. Passing it once the server honours it would
  // point the database at an empty directory and read as total data loss.
  const isDev = !!process.env.ELECTRON_RENDERER_URL

  let port: number

  if (isDev) {
    // Dev mode: use npx tsx to run TypeScript directly
    const repoRoot = path.join(__dirname, '../..')

    // A port for this launch only, which no stored setting could give. A dev
    // server and a packaged Vorn share one data directory and therefore one
    // remembered port, and the reason to set this is to make them differ.
    // Passed as `--port` rather than through the environment so it travels the
    // same path the CLI already takes and is refused the same way if malformed.
    const devPort = readDevPort()

    const child = spawn('npx', ['tsx', serverEntryPoint, ...(devPort ? ['--port', devPort] : [])], {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: {
        ...process.env,
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: process.env.NODE_ENV ?? 'development'
        // Connectors live in their own repository now, so a local build is
        // preferred by setting VORN_CONNECTORS_ROOT to that checkout. It
        // passes through with the rest of the environment above; deriving it
        // from this repo's root would point at packages that are not here.
      },
      cwd: repoRoot
    })

    child.stdin?.end()

    // Forward server stderr to our log
    if (child.stderr) {
      const errLines = createInterface({ input: child.stderr })
      errLines.on('line', (line) => {
        log.info(`[server] ${line}`)
      })
    }

    // Tracked before anything is awaited. `readServerPort` can reject on a ten
    // second timeout with the child still running, and a child assigned only
    // after that is one nothing holds a reference to -- unkillable by
    // `stopServer`, invisible to the relaunch logic, and still on the port.
    serverProcess = child
    child.on('exit', (code, signal) => {
      onServerExit(`code=${code}, signal=${signal}`)
    })

    port = await readServerPort(child).catch((err) => {
      child.kill('SIGKILL')
      throw err
    })
  } else {
    // Production: use Electron's utilityProcess.fork() to run the bundled
    // server as a proper Node.js child process (NOT another Electron instance)
    //
    // The main process has Electron's ASAR patching so it can resolve native
    // modules. The utilityProcess does NOT, so we resolve the absolute paths
    // here and pass them via environment variables for the server banner to use.
    const asarUnpacked = path.join(app.getAppPath() + '.unpacked', 'node_modules')

    const child = utilityProcess.fork(serverEntryPoint, [], {
      stdio: 'pipe',
      env: {
        ...process.env,
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: 'production',
        VORN_NATIVE_MODULES_PATH: asarUnpacked,
        NODE_PATH: [path.join(app.getAppPath(), 'node_modules'), asarUnpacked].join(path.delimiter)
      }
    })

    // Forward server stderr to our log
    if (child.stderr) {
      const errLines = createInterface({ input: child.stderr })
      errLines.on('line', (line) => {
        log.info(`[server] ${line}`)
      })
    }

    serverProcess = child
    child.on('exit', (code) => {
      onServerExit(`code=${code}`)
    })

    port = await readServerPort(child).catch((err) => {
      child.kill()
      throw err
    })
  }

  return port
}

/**
 * A server process has gone. Decide whether another should take its place.
 *
 * Recovery is only half missing: `ServerBridge` has always reconnected every two
 * seconds and never given up, so once something is listening again the app
 * reattaches itself. What was absent was anything to listen again -- this
 * handler logged the exit code and stopped, and a server that died took the
 * session with it until the app was quit and reopened.
 *
 * The bridge is kept and repointed rather than replaced, because `main` holds
 * the instance `launchServer` returned and would go on using the old one.
 */
function onServerExit(detail: string): void {
  const uptimeMs = lastSpawnAt === 0 ? 0 : Date.now() - lastSpawnAt
  log.warn(`[launcher] server exited (${detail}) after ${Math.round(uptimeMs / 1000)}s`)
  serverProcess = null
  relaunchAttempts = attemptsAfterExit(relaunchAttempts, uptimeMs)

  const decision = decideRelaunch({
    deliberate: stoppingDeliberately,
    hostMode: readHostSettings().mode === 'host',
    attempts: relaunchAttempts
  })

  if (!decision.relaunch) {
    log.warn(`[launcher] not restarting the server: ${decision.reason}`)
    return
  }

  relaunchAttempts += 1
  log.info(
    `[launcher] restarting the server in ${decision.delayMs}ms (attempt ${relaunchAttempts})`
  )

  relaunchTimer = setTimeout(() => {
    relaunchTimer = null
    // Asked again, because fifteen seconds is long enough for the answer to have
    // changed. The app may have begun quitting since, and spawning a server on
    // the way out is the one thing this must never do.
    if (stoppingDeliberately) {
      log.info('[launcher] not restarting after all: the app is shutting down')
      return
    }
    void spawnServer()
      .then((port) => {
        const url = `ws://127.0.0.1:${port}/ws`
        // Usually a no-op now that the port is remembered across restarts, which
        // is the case the reconnect loop already handles on its own. It matters
        // when the old port was taken in the moment between the two.
        if (bridge && bridge.target() !== url) {
          log.info(`[launcher] server came back on ${port}; repointing the bridge`)
          bridge.retarget(url)
        }
      })
      .catch((err) => {
        // Not re-entered here. The child now carries its own `exit` handler from
        // the moment it is spawned, so a failed start already routes back
        // through `onServerExit` -- calling it again would spend two attempts on
        // one failure.
        log.error({ err }, '[launcher] restart failed')
      })
  }, decision.delayMs)
}

export async function launchServer(): Promise<ServerBridge> {
  const host = readHostSettings()
  if (host.mode === 'host' && host.url) {
    // Configured for a host but holding no credential — safeStorage was
    // unavailable when it was saved, or the keychain has moved since. Falling
    // through to a local server would quietly open a different database than the
    // one the user asked for, and everything would look empty rather than wrong.
    // Failing here puts the connect window up asking for the token.
    if (!host.token) throw new Error(`No stored token for ${host.url}`)
    return connectToHost(host.url, host.token)
  }

  const port = await spawnServer()
  log.info(`[launcher] server started on port ${port}`)

  // Connect bridge
  bridge = new ServerBridge(`ws://127.0.0.1:${port}/ws`, bootstrapToken ?? undefined)
  bridge.connect()

  // Wait for connection
  await new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Bridge connection timeout')), 10_000)
    bridge!.once('connected', () => {
      clearTimeout(timeout)
      resolve()
    })
  })

  return bridge
}

export function getServerBridge(): ServerBridge | null {
  return bridge
}

export async function stopServer(): Promise<void> {
  // Before anything else: an exit we asked for must not look like a crash, and
  // a relaunch already counting down must not outlive the decision to quit.
  stoppingDeliberately = true
  if (relaunchTimer) {
    clearTimeout(relaunchTimer)
    relaunchTimer = null
  }

  if (bridge) {
    // Only ask a server to stop if this app is the one that started it.
    //
    // `serverProcess` is the test for that, and it is null in host mode. Sending
    // `server:shutdown` to a host would take the server down for everyone
    // connected to it — every other desktop, every phone — because one person
    // closed their laptop.
    if (serverProcess) {
      try {
        await bridge.request('server:shutdown', undefined, 5000)
      } catch {
        // Server may already be gone
      }
    }
    bridge.close()
    bridge = null
  }

  if (serverProcess) {
    if ('killed' in serverProcess) {
      // ChildProcess (dev mode)
      const child = serverProcess as ChildProcess
      if (!child.killed) {
        if (process.platform === 'win32') {
          child.kill()
        } else {
          child.kill('SIGTERM')
          setTimeout(() => {
            if (child && !child.killed) {
              child.kill('SIGKILL')
            }
          }, 3000)
        }
      }
    } else {
      // UtilityProcess (production) — only has kill()
      serverProcess.kill()
    }
    serverProcess = null
  }
}

function resolveServerEntry(): string {
  // In dev: packages/server/src/index.ts (run via tsx)
  // In production: resources/server/index.cjs (bundled)
  if (process.env.ELECTRON_RENDERER_URL) {
    // Dev mode — use tsx to run TypeScript directly
    return path.join(__dirname, '../../packages/server/src/index.ts')
  }
  return path.join(process.resourcesPath, 'server', 'index.cjs')
}

function readServerPort(child: ChildProcess | UtilityProcess): Promise<number> {
  return new Promise((resolve, reject) => {
    if (!child.stdout) {
      reject(new Error('No stdout on server process'))
      return
    }

    const rl = createInterface({ input: child.stdout })
    const timeout = setTimeout(() => {
      reject(new Error('Timeout waiting for server port'))
    }, 10_000)

    rl.on('line', (line) => {
      try {
        const parsed = JSON.parse(line)
        if (typeof parsed.port === 'number') {
          clearTimeout(timeout)
          rl.close()
          resolve(parsed.port)
        }
      } catch {
        // Not JSON or not our port line — ignore
      }
    })

    // Both ChildProcess and UtilityProcess support .on('exit', cb)
    const onExit = (code: number | null) => {
      clearTimeout(timeout)
      reject(new Error(`Server exited before reporting port (code=${code})`))
    }
    ;(child as UtilityProcess).on('exit', onExit)
  })
}
