import { spawn, type ChildProcess } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import path from 'node:path'
import { createInterface } from 'node:readline'
import { app } from 'electron'
import log from '../logger'
import { BOOTSTRAP_ENV_VAR, SERVER_PORT_ENV_VAR, type ServerHello } from '@vornrun/shared/protocol'
import { ServerBridge } from './server-bridge'
import { readHostSettings } from './host-store'
import { attemptsAfterExit, decideRelaunch } from './server-relaunch'
import {
  isPidAlive,
  judgeAdoption,
  readLocalToken,
  readPortFile,
  resolveDataDir,
  type AdoptionVerdict
} from './server-adoption'

let serverProcess: ChildProcess | null = null
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
 * The pid of a server we adopted rather than spawned.
 *
 * Recovery hangs off `child.on('exit')`, and an adopted server has no child to
 * emit one -- so without this, the one case the whole adoption path exists for
 * would be the one case that never recovers. The bridge would reconnect forever
 * against a port with nothing behind it, which is precisely the symptom #492
 * was written to end.
 */
let adoptedPid: number | null = null

/** What the last adoption attempt refused, for the UI to explain. Null when none did. */
let lastRefusal: (AdoptionVerdict & { kind: 'refuse' }) | null = null

export function getLastAdoptionRefusal(): (AdoptionVerdict & { kind: 'refuse' }) | null {
  return lastRefusal
}

/** `dev` or `packaged`, decided the same way the server entry point is resolved. */
function buildChannel(): 'dev' | 'packaged' {
  return process.env.ELECTRON_RENDERER_URL ? 'dev' : 'packaged'
}

/**
 * What the server needs to describe itself in `server:hello`.
 *
 * Passed rather than inferred server-side because only this process knows for
 * certain which build spawned it; the server's own fallback reads its entry
 * extension, which is right for a CLI run and merely probable here.
 */
function identityEnv(): Record<string, string> {
  return { VORN_BUILD_CHANNEL: buildChannel(), VORN_APP_VERSION: app.getVersion() }
}

/**
 * Let go of a detached child's pipes once it has reported for duty.
 *
 * Both are `Socket`s, and an open one references this process's event loop --
 * `child.unref()` does not cover them, so leaving either attached keeps Electron
 * alive at quit waiting on a server that is never going to close them.
 *
 * They exist at all for the reason abduco keeps a pipe across its own fork: a
 * daemon that fails during startup has nowhere to say so, and a silent failure
 * to start is far worse to debug than a noisy one. So they stay open exactly as
 * long as the answer takes.
 */
function releaseChildStreams(child: ChildProcess): void {
  child.stdout?.destroy()
  child.stderr?.destroy()
}

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
 * Both modes spawn detached, so the server outlives this app: quitting Vorn is a
 * window closing, not every agent dying. Dev runs `npx tsx` over the TypeScript
 * source; production spawns the Electron binary with ELECTRON_RUN_AS_NODE=1,
 * which yields a plain Node process from the same signed bundle.
 *
 * NOTE: `process.execPath` is the Electron binary, and spawning it *without*
 * ELECTRON_RUN_AS_NODE launches another full Electron app — an infinite spawn
 * loop this code has hit before. The variable is what makes it a Node process,
 * so it is not optional.
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
      stdio: ['ignore', 'pipe', 'pipe'],
      // Its own process group and session, so it survives this app -- and so a
      // Ctrl-C aimed at the terminal running `yarn dev` does not reach it.
      detached: true,
      env: {
        ...process.env,
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: process.env.NODE_ENV ?? 'development',
        ...identityEnv()
        // Connectors live in their own repository now, so a local build is
        // preferred by setting VORN_CONNECTORS_ROOT to that checkout. It
        // passes through with the rest of the environment above; deriving it
        // from this repo's root would point at packages that are not here.
      },
      cwd: repoRoot
    })

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
    releaseChildStreams(child)
    child.unref()
  } else {
    // Deliberately NOT utilityProcess.fork: a utility process is tied to this
    // app's lifetime by design, so it can never outlive the window -- which is
    // the whole point here. Spawning the Electron binary with
    // ELECTRON_RUN_AS_NODE gives a plain Node process from the same signed
    // bundle, which the hardened runtime already permits because
    // `resources/entitlements.mac.plist` grants
    // com.apple.security.cs.allow-dyld-environment-variables.
    //
    // The variable is not optional: process.execPath is the Electron binary, and
    // spawning it without this launches a second full Vorn -- an infinite spawn
    // loop this code has hit before.
    //
    // The main process has Electron's ASAR patching so it can resolve native
    // modules. A plain Node child does NOT, so the absolute paths are resolved
    // here and passed through the environment for the server to use.
    const asarUnpacked = path.join(app.getAppPath() + '.unpacked', 'node_modules')

    const child = spawn(process.execPath, [serverEntryPoint], {
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
      // A daemon must not hold open a directory that can be deleted underneath
      // it. The data dir is the one place guaranteed to exist for as long as the
      // server has anything to serve.
      cwd: resolveDataDir(),
      env: {
        ...process.env,
        ELECTRON_RUN_AS_NODE: '1',
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: 'production',
        VORN_NATIVE_MODULES_PATH: asarUnpacked,
        NODE_PATH: [path.join(app.getAppPath(), 'node_modules'), asarUnpacked].join(path.delimiter),
        ...identityEnv()
      }
    })

    serverProcess = child
    child.on('exit', (code) => {
      onServerExit(`code=${code}`)
    })

    port = await readServerPort(child).catch((err) => {
      child.kill('SIGKILL')
      throw err
    })
    releaseChildStreams(child)
    child.unref()
  }

  // A server we started is one we supervise through its child handle, so any
  // earlier adoption no longer describes what is running.
  adoptedPid = null
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

/** Long enough for a busy event loop, short enough not to stall a cold launch. */
const ADOPT_HELLO_TIMEOUT_MS = 3_000
const ADOPT_CONNECT_TIMEOUT_MS = 5_000

/**
 * Connect to a server that is already running and decide whether to keep it.
 *
 * Never kills the incumbent. The process holding the PTYs is the one with the
 * user's work in it, so a client that cannot speak to it declines and says so
 * rather than settling the disagreement by force -- the rule tmux and zellij
 * both landed on. Refusing costs a spare process; killing costs the session.
 */
async function tryAdopt(
  port: number,
  self: { dataDir: string; buildChannel: 'dev' | 'packaged' }
): Promise<{ bridge: ServerBridge } | { refusal: AdoptionVerdict & { kind: 'refuse' } } | null> {
  const token = readLocalToken(self.dataDir)
  if (!token) {
    log.info('[launcher] a port is published but no credential is; starting our own')
    return null
  }

  const candidate = new ServerBridge(`ws://127.0.0.1:${port}/ws`, token)
  candidate.connect()

  // The greeting is the first frame on the socket and arrives before
  // authentication, so this resolves even against a server that would reject the
  // credential. A timeout means "cannot tell", which must not be read as "dead":
  // that reading is how a second server gets started on a live port.
  const hello = await new Promise<ServerHello | null>((resolve) => {
    const timer = setTimeout(() => resolve(null), ADOPT_HELLO_TIMEOUT_MS)
    candidate.once('hello', (h: ServerHello) => {
      clearTimeout(timer)
      resolve(h)
    })
  })

  const verdict = judgeAdoption(hello, self)
  if (verdict.kind === 'refuse') {
    candidate.close()
    return { refusal: verdict }
  }

  // Judged, but not yet usable: a rejected credential closes the socket at 4001
  // rather than failing the connect, so the greeting alone proves nothing about
  // whether this app may actually talk to it.
  const usable = await new Promise<boolean>((resolve) => {
    if (candidate.isConnected) return resolve(true)
    const timer = setTimeout(() => resolve(false), ADOPT_CONNECT_TIMEOUT_MS)
    candidate.once('connected', () => {
      clearTimeout(timer)
      resolve(true)
    })
  })
  if (!usable) {
    candidate.close()
    log.warn('[launcher] the running server did not accept this app; starting our own')
    return null
  }

  adoptedPid = hello?.pid ?? null
  // An adopted server has no child handle, so nothing would ever call
  // `onServerExit` for it. Watching the socket is the only signal available, and
  // the pid distinguishes the two reasons it drops: a server that died, and one
  // that is merely busy while the bridge retries.
  candidate.on('disconnected', () => {
    if (adoptedPid === null || stoppingDeliberately) return
    if (isPidAlive(adoptedPid)) return
    const pid = adoptedPid
    adoptedPid = null
    onServerExit(`adopted server pid=${pid} is gone`)
  })

  log.info(`[launcher] adopted the server already running on port ${port}`)
  return { bridge: candidate }
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

  // Before spawning anything: is one already running? The ordinary path now that
  // the server outlives the app, not an edge case. Skipping it would leave the
  // old server holding the PTYs while this one took another port -- two servers
  // on one database, the app talking to the empty one.
  const dataDir = resolveDataDir()
  lastRefusal = null
  const published = readPortFile(dataDir)
  if (published) {
    const outcome = await tryAdopt(published.port, { dataDir, buildChannel: buildChannel() })
    if (outcome && 'bridge' in outcome) {
      bridge = outcome.bridge
      return bridge
    }
    if (outcome && 'refusal' in outcome) {
      lastRefusal = outcome.refusal
      log.warn(
        `[launcher] declined to adopt the running server (${outcome.refusal.reason}): ` +
          `${outcome.refusal.detail}. It keeps running and keeps its sessions.`
      )
    }
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

/**
 * Let go of the server without stopping it.
 *
 * The ordinary quit path once sessions outlive the window: close the socket and
 * leave the process alone. Deliberately not `stopServer` with a flag -- the two
 * read identically at the call site and mean opposite things to the person whose
 * agent is mid-turn, so they are separate names.
 */
export function detachFromServer(): void {
  // Same first move as `stopServer`, for the same reason: the socket is about to
  // drop, and neither the exit handler nor a countdown already running may take
  // that as a crash worth answering with a new server on the way out.
  stoppingDeliberately = true
  if (relaunchTimer) {
    clearTimeout(relaunchTimer)
    relaunchTimer = null
  }

  bridge?.close()
  bridge = null
  // Dropped without killing. The child is unref'd and in its own process group,
  // so nothing here holds it and nothing here will end it.
  serverProcess = null
  adoptedPid = null
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
    //
    // An adopted server counts as ours: it is on this machine, on this data
    // directory, and this app is the one being asked to stop everything. It has
    // no child handle, so the RPC is the only way to end it.
    if (serverProcess || adoptedPid !== null) {
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
    const child = serverProcess
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
    serverProcess = null
  }
  adoptedPid = null
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

/**
 * Wait for the server to say which port it took.
 *
 * The pipe is the primary channel and the only one that can also carry a startup
 * crash, which is why a detached child keeps one. Its stderr is drained into our
 * log for the same window -- a server that dies during startup says why here or
 * nowhere -- and both are released the moment the answer arrives.
 *
 * The port file is the fallback rather than belt-and-braces: the server writes
 * `{port, pid}` there for MCP already, so a launch that lost the pipe can still
 * find a healthy server instead of concluding it failed and spawning a second.
 */
function readServerPort(child: ChildProcess): Promise<number> {
  return new Promise((resolve, reject) => {
    if (!child.stdout) {
      reject(new Error('No stdout on server process'))
      return
    }

    if (child.stderr) {
      const errLines = createInterface({ input: child.stderr })
      errLines.on('line', (line) => log.info(`[server] ${line}`))
    }

    const rl = createInterface({ input: child.stdout })
    const settle = (fn: () => void): void => {
      clearTimeout(timeout)
      rl.close()
      fn()
    }

    const timeout = setTimeout(() => {
      const published = readPortFile()
      settle(() =>
        published ? resolve(published.port) : reject(new Error('Timeout waiting for server port'))
      )
    }, 10_000)

    rl.on('line', (line) => {
      try {
        const parsed = JSON.parse(line)
        if (typeof parsed.port === 'number') {
          settle(() => resolve(parsed.port))
        }
      } catch {
        // Not JSON or not our port line — ignore
      }
    })

    child.on('exit', (code) => {
      settle(() => reject(new Error(`Server exited before reporting port (code=${code})`)))
    })
  })
}
