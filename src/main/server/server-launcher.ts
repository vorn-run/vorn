import { spawn, type ChildProcess } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { closeSync, mkdirSync, openSync } from 'node:fs'
import path, { join } from 'node:path'
import { createInterface } from 'node:readline'
import { app } from 'electron'
import log from '../logger'
import {
  BOOTSTRAP_ENV_VAR,
  SERVER_PORT_ENV_VAR,
  type ServerIdentity
} from '@vornrun/shared/protocol'
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

/**
 * Thrown when a server is running that this app may not use.
 *
 * Not an ordinary failure: nothing is broken, and the user's agents are alive
 * and working inside it. It carries the reason so the window that comes up can
 * say which of the cases it was.
 */
export class AdoptionRefusedError extends Error {
  constructor(readonly refusal: AdoptionVerdict & { kind: 'refuse' }) {
    super(`Another Vorn server is running that this app cannot use: ${refusal.detail}`)
    this.name = 'AdoptionRefusedError'
  }
}

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

/**
 * What the last adoption attempt refused, and the pid still holding the sessions.
 *
 * Kept because a refusal is the user's problem, not just a log line: their agents
 * are alive in a server this app cannot speak to, and the app they are looking at
 * is empty. The pid is what lets them end it on purpose -- the one actor allowed
 * to, since this code never ends an incumbent on its own.
 */
export type AdoptionRefusal = AdoptionVerdict & { kind: 'refuse' } & { incumbentPid: number | null }

let lastRefusal: AdoptionRefusal | null = null

export function getLastAdoptionRefusal(): AdoptionRefusal | null {
  return lastRefusal
}

/**
 * End the local server, because the user asked.
 *
 * The doctrine is that a starting app never ends a running server -- it holds
 * somebody's work and cannot be judged from outside. It says nothing about the
 * person whose work it is, who is the only one in a position to decide.
 *
 * Reads the pid from the port file rather than from any greeting: it is the one
 * value written by a process this app can attribute, and it is the same answer
 * whether the caller is the connect window (a server it refused to adopt) or the
 * running app (a local server still going while this is pointed at a host).
 */
export async function stopLocalServer(): Promise<{ ok: true } | { ok: false; error: string }> {
  const published = readPortFile()
  if (!published?.pid) return { ok: false, error: 'No local server is running.' }
  const pid = published.pid
  try {
    process.kill(pid, 'SIGTERM')
  } catch (err) {
    log.warn({ err }, '[launcher] could not signal the local server')
    return { ok: false, error: 'Could not stop it. It may have already exited.' }
  }

  // Waited for, not assumed. The caller relaunches the app the moment this says
  // yes, and a server still shutting down still owns its ws-port file -- so the
  // fresh launch would find it, refuse it again, and land straight back in the
  // window the user just acted from. A loop that looks like the button doing
  // nothing.
  if (await waitForExit(pid, STOP_GRACE_MS)) {
    log.info(`[launcher] stopped the local server (pid ${pid}) at the user's request`)
    lastRefusal = null
    return { ok: true }
  }

  try {
    log.warn(`[launcher] local server ${pid} ignored SIGTERM; sending SIGKILL`)
    process.kill(pid, 'SIGKILL')
  } catch {
    /* it may have exited in the meantime, which is the outcome we wanted */
  }
  if (await waitForExit(pid, STOP_KILL_GRACE_MS)) {
    lastRefusal = null
    return { ok: true }
  }
  return { ok: false, error: 'It is still running and did not respond to being stopped.' }
}

/** How long a server gets to leave politely, and then to leave at all. */
const STOP_GRACE_MS = 5_000
const STOP_KILL_GRACE_MS = 2_000

/** Resolves true once the pid is gone, false if it outlasts the deadline. */
async function waitForExit(pid: number, deadlineMs: number): Promise<boolean> {
  const until = Date.now() + deadlineMs
  while (Date.now() < until) {
    if (!isPidAlive(pid)) return true
    await new Promise((r) => setTimeout(r, 100))
  }
  return !isPidAlive(pid)
}

/**
 * The data directory, created if this is the first run.
 *
 * Normally the server makes it (`database.ts` on init), but it is also the cwd
 * the server is spawned into, and that has to exist before the spawn rather than
 * after it.
 */
function ensureDataDir(): string {
  const dir = resolveDataDir()
  mkdirSync(dir, { recursive: true, mode: 0o700 })
  return dir
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
  const isDev = buildChannel() === 'dev'

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
      // Deliberately NOT detached, where production is.
      //
      // A detached dev server outlives `yarn dev`, and the next `yarn dev` would
      // adopt it -- same data directory, same build channel, so every check
      // passes. It would be running the source as it was before the edit that
      // prompted the restart, and nothing would say so. An hour lost to a fix
      // that "did not work" is a worse trade than restarting sessions a
      // developer was going to restart anyway.
      //
      // Sessions surviving a quit is a property of the shipped app. Dev keeps
      // the old lifetime, and the buildChannel check still earns its place: a
      // packaged server does outlive its app, and this is what stops `yarn dev`
      // from adopting it.
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
    child.on('exit', onChildExit)

    port = await readServerPort(child).catch((err) => {
      child.kill('SIGKILL')
      throw err
    })
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
    // Created here rather than at the top of this function: only this branch
    // reads it, and the dev branch would otherwise pay a blocking recursive
    // mkdirSync on every spawn -- including every crash relaunch during a
    // `yarn dev` session -- for a value it never touches.
    const dataDir = ensureDataDir()
    const asarUnpacked = path.join(app.getAppPath() + '.unpacked', 'node_modules')

    // Straight to a file, not to pipes.
    //
    // A pipe held by this process dies with this process, and the server is
    // meant to outlive it: the next write to stderr after that raises EPIPE, and
    // the server installs no uncaughtException handler, so it exits. Its logger
    // writes to `process.stderr` on every request, so the window is one log line
    // wide. Measured both ways -- destroying the parent's end and merely
    // unref'ing it both killed the child on its next write.
    //
    // A file descriptor does not care that the parent is gone, and the startup
    // diagnostics a pipe existed to carry end up somewhere durable rather than
    // in a log that stops the moment the app does.
    const logFd = openSync(join(dataDir, SERVER_LOG_FILENAME), 'a')
    const child = spawn(process.execPath, [serverEntryPoint], {
      stdio: ['ignore', logFd, logFd],
      detached: true,
      // A daemon must not hold open a directory that can be deleted underneath
      // it, so not the app bundle and not a worktree. The data dir is where the
      // server's own files live, so it lasts as long as the server has anything
      // to serve -- but on a first run it does not exist yet: the *server*
      // creates it, and it has not started. `spawn` with a missing cwd fails
      // ENOENT, so it is created here first.
      cwd: dataDir,
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

    // Closed here as soon as the child holds its own copy; leaving it open would
    // keep a descriptor in this process for a file only the server writes to.
    closeSync(logFd)

    serverProcess = child
    child.on('exit', onChildExit)
    child.on('error', (err) => {
      log.error({ err }, '[launcher] could not start the server')
    })

    // Nothing to read from: the port arrives through the file the server writes
    // for MCP, and the same watcher that waits for it proves the server got as
    // far as listening.
    port = await waitForPublishedPort(child).catch((err) => {
      child.kill('SIGKILL')
      throw err
    })
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
  // Both, not just the child handle. "How are we holding this server" is one
  // fact stored in two variables -- a child we spawned, or a pid we adopted --
  // and until this cleared only one of them, the caller on the adopted path had
  // to clear the other itself. One invariant with two owners is how they drift.
  serverProcess = null
  adoptedPid = null
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

/**
 * Long enough for a busy event loop, short enough not to stall a cold launch.
 *
 * A server that sends no identity frame at all -- too old, or not on loopback --
 * costs exactly this before the launch gives up and spawns its own.
 */
const ADOPT_IDENTITY_TIMEOUT_MS = 3_000
/** How long to wait for the round trip that proves the credential was accepted. */
const ADOPT_AUTH_TIMEOUT_MS = 5_000

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
  expectedPid: number | undefined,
  self: { dataDir: string; buildChannel: 'dev' | 'packaged' }
): Promise<{ bridge: ServerBridge } | { refusal: AdoptionVerdict & { kind: 'refuse' } }> {
  const token = readLocalToken(self.dataDir)
  if (!token) {
    return {
      refusal: {
        kind: 'refuse',
        reason: 'unusable',
        detail: 'a server is listening but published no credential to reach it with'
      }
    }
  }

  const candidate = new ServerBridge(`ws://127.0.0.1:${port}/ws`, token)
  candidate.connect()

  // The greeting is the first frame on the socket and arrives before
  // authentication, so this resolves even against a server that would reject the
  // credential. A timeout means "cannot tell", which must not be read as "dead":
  // that reading is how a second server gets started on a live port.
  const identity = await new Promise<ServerIdentity | null>((resolve) => {
    const timer = setTimeout(() => resolve(null), ADOPT_IDENTITY_TIMEOUT_MS)
    candidate.once('identity', (found: ServerIdentity) => {
      clearTimeout(timer)
      resolve(found)
    })
  })

  const verdict = judgeAdoption(identity, candidate.serverHelloVersion, {
    ...self,
    expectedPid
  })
  if (verdict.kind === 'refuse') {
    candidate.close()
    return { refusal: verdict }
  }

  // Judged, but not yet known to be usable. A rejected credential does not fail
  // the connect: the socket opens, the greeting arrives, and only then does the
  // server close it at 4001/4002. So `isConnected` is true by the time the
  // greeting resolves and proves nothing -- an earlier version of this checked
  // exactly that and could never have been false.
  //
  // A timer would only guess. An authenticated round trip is the question
  // itself: `config:load` is refused outright without a credential, so a reply
  // to it *is* the proof, and it is a read this app makes moments later anyway.
  const accepted = await candidate
    .request('config:load', undefined, ADOPT_AUTH_TIMEOUT_MS)
    .then(() => true)
    .catch(() => false)
  if (!accepted) {
    candidate.close()
    return {
      refusal: {
        kind: 'refuse',
        reason: 'unusable',
        detail: 'the running server did not accept this app'
      }
    }
  }

  // Carried forward, not discarded. If this server later dies and a replacement
  // is spawned, `spawnServer` reuses this value rather than minting a fresh one
  // -- and the bridge is retargeted, never rebuilt, so its credential is fixed
  // at construction. Leaving it null meant the replacement got a new secret the
  // surviving bridge had never heard of, and the bridge would then reconnect
  // forever against a healthy server, rejected every time.
  bootstrapToken = token

  adoptedPid = expectedPid ?? null
  // An adopted server has no child handle, so nothing would ever call
  // `onServerExit` for it. Watching the socket is the only signal available, and
  // the pid distinguishes the two reasons it drops: a server that died, and one
  // that is merely busy while the bridge retries.
  candidate.on('disconnected', () => {
    if (adoptedPid === null || stoppingDeliberately) return
    if (isPidAlive(adoptedPid)) return
    onServerExit(`adopted server pid=${adoptedPid} is gone`)
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
    const outcome = await tryAdopt(published.port, published.pid, {
      dataDir,
      buildChannel: buildChannel()
    })
    if ('bridge' in outcome) {
      bridge = outcome.bridge
      return bridge
    }
    const { refusal } = outcome
    lastRefusal = { ...refusal, incumbentPid: published.pid ?? null }
    log.warn(
      `[launcher] declined to adopt the running server (${refusal.reason}): ` +
        `${refusal.detail}. It keeps running and keeps its sessions.`
    )
    // Declining to adopt is not a reason to start a rival.
    //
    // Both servers would open the same SQLite file, and `saveSessions` is a
    // DELETE-then-insert of the whole table on a debounce -- so the two would
    // erase each other's sessions, last writer winning, and the app would show
    // whichever set survived. The second would also lose the ws-port file to the
    // incumbent's liveness guard, making it invisible to MCP.
    //
    // This is the half of the doctrine that is easy to drop. tmux does not kill a
    // server it cannot speak to, and it does not start one beside it either: the
    // *client* prints the mismatch and exits. There is nothing useful this app can
    // do against a database another server is holding, so it says so and stops,
    // and the person decides which server should live.
    throw new AdoptionRefusedError(refusal)
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
/**
 * Stop treating the server's departure as a failure.
 *
 * The first move of every deliberate ending, whether the server is being killed
 * or merely let go of: the socket is about to drop, and neither the exit handler
 * nor a countdown already running may read that as a crash worth answering with
 * a new server on the way out.
 */
function beginDeliberateStop(): void {
  stoppingDeliberately = true
  if (relaunchTimer) {
    clearTimeout(relaunchTimer)
    relaunchTimer = null
  }
}

export function detachFromServer(): void {
  beginDeliberateStop()

  bridge?.close()
  bridge = null

  // In dev the child is not detached, and on POSIX nothing kills a plain child
  // when its parent exits -- it is simply reparented. Walking away would leave
  // the `npx tsx` server running for the next `yarn dev` to adopt, which is the
  // stale-source trap the dev spawn is written to avoid. Dev keeps the old
  // lifetime in full: the server goes when the app goes.
  if (serverProcess && buildChannel() === 'dev') {
    serverProcess.kill('SIGTERM')
  }

  // Otherwise dropped without killing: a packaged child is unref'd and in its
  // own process group, so nothing here holds it and nothing here will end it.
  serverProcess = null
  adoptedPid = null
}

export async function stopServer(): Promise<void> {
  beginDeliberateStop()

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
    if (process.platform === 'win32') {
      child.kill()
    } else {
      child.kill('SIGTERM')
      setTimeout(() => {
        // Asked whether the process is still there, not whether we asked it to
        // go. `child.killed` is true the moment a signal is *delivered*, even to
        // a process that ignores it -- so gating on it made this fallback
        // unreachable and left a server running after the user chose to stop it.
        // That was survivable while the server died with its parent anyway; a
        // detached one just keeps going.
        if (child.pid && isPidAlive(child.pid)) {
          log.warn(`[launcher] server ${child.pid} ignored SIGTERM; sending SIGKILL`)
          child.kill('SIGKILL')
        }
      }, 3000)
    }
    serverProcess = null
  } else if (adoptedPid !== null && isPidAlive(adoptedPid)) {
    // An adopted server has no child handle, so the RPC above was the only way
    // to ask -- and if it threw or timed out, nothing has happened yet. Without
    // this the user picks "Stop Sessions and Server", the app quits, and every
    // session carries on with nothing to say so. The pid is the handle left.
    log.warn('[launcher] the adopted server did not answer; signalling it directly')
    try {
      process.kill(adoptedPid, 'SIGTERM')
    } catch (err) {
      log.warn({ err }, '[launcher] could not signal the adopted server')
    }
  }
  adoptedPid = null
}

function resolveServerEntry(): string {
  // In dev: packages/server/src/index.ts (run via tsx)
  // In production: resources/server/index.cjs (bundled)
  if (buildChannel() === 'dev') {
    // Dev mode — use tsx to run TypeScript directly
    return path.join(__dirname, '../../packages/server/src/index.ts')
  }
  return path.join(process.resourcesPath, 'server', 'index.cjs')
}

/**
 * Wait for a server we just spawned to publish its port.
 *
 * Only for the packaged branch, which spawns the server directly. Dev goes
 * through `npx tsx`, so its child is npx -- a parent of the process that
 * actually listens -- and the pid in the port file is the server's, never the
 * child's. Merging the two readiness paths onto this one therefore cannot work:
 * in dev the match below would never be satisfied and the launch would hang for
 * the full timeout. `tests/server-port-stability.test.ts` runs the tsx binary
 * directly for the same reason.
 *
 * Only ever satisfied by a file naming *this* child. The same file is what
 * `launchServer` reads before deciding whether to adopt, so a launch that
 * declined an incumbent -- wrong protocol, wrong build -- would otherwise find
 * the incumbent's record still sitting there and resolve to its port, handing
 * the bridge a credential that server never had while the child we actually
 * started ran unwatched somewhere else.
 */
function waitForPublishedPort(child: ChildProcess): Promise<number> {
  const deadline = Date.now() + SPAWN_READY_TIMEOUT_MS
  return new Promise((resolve, reject) => {
    let exited: number | null = null
    child.once('exit', (code) => {
      exited = code ?? -1
    })

    const poll = (): void => {
      const published = readPortFile()
      if (published && published.pid === child.pid) {
        resolve(published.port)
        return
      }
      if (exited !== null) {
        reject(new Error(`Server exited before reporting port (code=${exited})`))
        return
      }
      if (Date.now() >= deadline) {
        reject(
          new Error(
            `Timeout waiting for the server to start. Its output is in ${SERVER_LOG_FILENAME}.`
          )
        )
        return
      }
      setTimeout(poll, SPAWN_POLL_MS).unref?.()
    }
    poll()
  })
}

/**
 * One handler for both branches, so a packaged crash is logged as fully as a dev
 * one. The signal is the interesting half when a server is killed rather than
 * exiting, and the production branch used to drop it for no reason anyone wrote
 * down -- which is precisely the diagnostic a detached server most needs.
 */
function onChildExit(code: number | null, signal: NodeJS.Signals | null): void {
  onServerExit(`code=${code}, signal=${signal}`)
}

/** Where a detached server's stdout and stderr go, under the data directory. */
const SERVER_LOG_FILENAME = 'server.log'
const SPAWN_READY_TIMEOUT_MS = 20_000
const SPAWN_POLL_MS = 100

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

    // A spawn that never starts emits `error`, not `exit`, and an EventEmitter
    // with nothing listening for `error` throws -- taking the main process down
    // instead of reporting a server that failed to launch.
    //
    // Registered below `settle` rather than above it. Node emits this one
    // asynchronously, so the earlier order was safe -- but it was safe only
    // because of when the event fires, which is a thing to know rather than a
    // thing to read.
    child.on('error', (err) => {
      settle(() => reject(err))
    })

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
