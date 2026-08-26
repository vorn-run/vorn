import { spawn, type ChildProcess } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import path from 'node:path'
import { createInterface } from 'node:readline'
import { app } from 'electron'
import log from '../logger'
import { BOOTSTRAP_ENV_VAR, SERVER_PORT_ENV_VAR, type ServerHello } from '@vornrun/shared/protocol'
import { ServerBridge } from './server-bridge'
import { readHostSettings } from './host-store'
import {
  judgeAdoption,
  readLocalToken,
  readPortFile,
  resolveDataDir,
  type AdoptionVerdict
} from './server-adoption'

let serverProcess: ChildProcess | null = null
let bridge: ServerBridge | null = null

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
 * loop this code has hit before. The env var is what makes it a Node process, so
 * it is not optional.
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
 * What the server needs to describe itself in `server:hello`.
 *
 * The channel is passed rather than inferred server-side because only this
 * process knows for certain which build spawned it — the server's own fallback
 * reads its entry extension, which is right for a CLI run and merely probable
 * here.
 */
function identityEnv(buildChannel: 'dev' | 'packaged'): Record<string, string> {
  return {
    VORN_BUILD_CHANNEL: buildChannel,
    VORN_APP_VERSION: app.getVersion()
  }
}

/**
 * Let go of a detached child's pipes once it has reported for duty.
 *
 * Both are `Socket`s, and an open one references this process's event loop —
 * `child.unref()` does not cover them, so leaving either attached would keep
 * Electron alive at quit waiting on a server that is never going to close them.
 *
 * The pipes exist at all for the reason abduco keeps one across its own fork
 * (`~/dev/references/abduco/abduco.c:413-480`): a daemon that fails during
 * startup has nowhere to say so, and a silent failure to start is far worse to
 * debug than a noisy one. So they stay open exactly as long as the answer takes.
 */
function releaseChildStreams(child: ChildProcess): void {
  child.stdout?.destroy()
  child.stderr?.destroy()
}

/**
 * Connect to a server that is already running and decide whether to keep it.
 *
 * Returns the bridge on adoption, or null with the reason recorded. Never kills
 * the incumbent: the process holding the PTYs is the one with the user's work in
 * it, so a client that cannot speak to it declines rather than resolving the
 * disagreement by force. The caller then spawns its own, which is a wasted
 * process at worst — where killing would be lost work.
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
  // credential. A timeout here means "cannot tell", which must not be read as
  // "dead" — we decline and spawn rather than assuming the port is free.
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

  // Adopted, but not yet usable: `connect` fires before the socket opens, and a
  // rejected credential closes it at 4001 rather than failing the connect.
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

  log.info(`[launcher] adopted the server already running on port ${port}`)
  return { bridge: candidate }
}

/** Long enough for a busy event loop, short enough not to stall a cold launch. */
const ADOPT_HELLO_TIMEOUT_MS = 3_000
const ADOPT_CONNECT_TIMEOUT_MS = 5_000

/** What the last adoption attempt refused, for the UI to explain. Null when none did. */
let lastRefusal: (AdoptionVerdict & { kind: 'refuse' }) | null = null

export function getLastAdoptionRefusal(): (AdoptionVerdict & { kind: 'refuse' }) | null {
  return lastRefusal
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

  // Before spawning anything: is one already running? This is the ordinary path
  // now that the server outlives the app, not an edge case.
  const dataDir = resolveDataDir()
  const buildChannel: 'dev' | 'packaged' = process.env.ELECTRON_RENDERER_URL ? 'dev' : 'packaged'
  lastRefusal = null
  const published = readPortFile(dataDir)
  if (published) {
    const outcome = await tryAdopt(published.port, { dataDir, buildChannel })
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

  const serverEntryPoint = resolveServerEntry()
  log.info(`[launcher] starting server: ${serverEntryPoint}`)

  // A per-launch credential for the server we are about to spawn. Generated here
  // rather than persisted: nothing to leak at rest, and it dies with the process.
  // The name is stripped unconditionally by `filterEnv`, so it never reaches a
  // PTY, headless agent or script node.
  const bootstrapToken = randomBytes(32).toString('base64url')

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
      // Its own process group and session, so it survives this app and so a
      // Ctrl-C aimed at the terminal running `yarn dev` does not reach it.
      detached: true,
      env: {
        ...process.env,
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: process.env.NODE_ENV ?? 'development',
        ...identityEnv(buildChannel)
        // Connectors live in their own repository now, so a local build is
        // preferred by setting VORN_CONNECTORS_ROOT to that checkout. It
        // passes through with the rest of the environment above; deriving it
        // from this repo's root would point at packages that are not here.
      },
      cwd: repoRoot
    })

    port = await readServerPort(child)
    releaseChildStreams(child)
    child.unref()

    child.on('exit', (code, signal) => {
      log.warn(`[launcher] server exited (code=${code}, signal=${signal})`)
      serverProcess = null
    })

    serverProcess = child
  } else {
    // The main process has Electron's ASAR patching so it can resolve native
    // modules. A plain Node child does NOT, so the absolute paths are resolved
    // here and passed through the environment for the server to use.
    const asarUnpacked = path.join(app.getAppPath() + '.unpacked', 'node_modules')

    // Deliberately NOT utilityProcess.fork: a utility process is tied to this
    // app's lifetime by design, so it can never outlive the window. Spawning the
    // Electron binary with ELECTRON_RUN_AS_NODE gives a plain Node process from
    // the same signed bundle — the hardened runtime already allows it, because
    // `resources/entitlements.mac.plist` grants
    // com.apple.security.cs.allow-dyld-environment-variables.
    const child = spawn(process.execPath, [serverEntryPoint], {
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
      // A daemon must not hold a directory open that can be deleted underneath
      // it. The data dir is the one place guaranteed to exist for as long as the
      // server has anything to serve.
      cwd: dataDir,
      env: {
        ...process.env,
        ELECTRON_RUN_AS_NODE: '1',
        [BOOTSTRAP_ENV_VAR]: bootstrapToken,
        NODE_ENV: 'production',
        VORN_NATIVE_MODULES_PATH: asarUnpacked,
        NODE_PATH: [path.join(app.getAppPath(), 'node_modules'), asarUnpacked].join(path.delimiter),
        ...identityEnv(buildChannel)
      }
    })

    port = await readServerPort(child)
    releaseChildStreams(child)
    child.unref()

    child.on('exit', (code) => {
      log.warn(`[launcher] server exited (code=${code})`)
      serverProcess = null
    })

    serverProcess = child
  }

  log.info(`[launcher] server started on port ${port}`)

  // Connect bridge
  bridge = new ServerBridge(`ws://127.0.0.1:${port}/ws`, bootstrapToken)
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
 * leave the process alone. Deliberately not `stopServer` with a flag — the two
 * read identically at the call site and mean opposite things to the person whose
 * agent is mid-turn, so they are separate names.
 */
export function detachFromServer(): void {
  bridge?.close()
  bridge = null
  // Dropped without killing. The child is already unref'd and in its own process
  // group, so nothing here is holding it and nothing here will end it.
  serverProcess = null
}

export async function stopServer(): Promise<void> {
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
 * The pipe is the primary channel and the only one that can also report a
 * startup crash, which is why it exists on an otherwise detached child. Its
 * stderr is drained into our log for the same window — a server that dies during
 * startup says why here or nowhere.
 *
 * The port file is the fallback, for the case the pipe is closed or the line is
 * missed. It is not merely belt-and-braces: the server writes `{port, pid}` there
 * for MCP already, so a launch that lost the pipe can still find a healthy server
 * rather than concluding it failed.
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
