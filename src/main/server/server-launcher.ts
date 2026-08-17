import { spawn, type ChildProcess } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import path from 'node:path'
import { createInterface } from 'node:readline'
import { app, utilityProcess, type UtilityProcess } from 'electron'
import log from '../logger'
import { BOOTSTRAP_ENV_VAR } from '@vornrun/shared/protocol'
import { ServerBridge } from './server-bridge'

let serverProcess: ChildProcess | UtilityProcess | null = null
let bridge: ServerBridge | null = null

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
export async function launchServer(): Promise<ServerBridge> {
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

    const child = spawn('npx', ['tsx', serverEntryPoint], {
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

    port = await readServerPort(child)

    child.on('exit', (code, signal) => {
      log.warn(`[launcher] server exited (code=${code}, signal=${signal})`)
      serverProcess = null
    })

    serverProcess = child
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

    port = await readServerPort(child)

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

export async function stopServer(): Promise<void> {
  if (bridge) {
    try {
      await bridge.request('server:shutdown', undefined, 5000)
    } catch {
      // Server may already be gone
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
