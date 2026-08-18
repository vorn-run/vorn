import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { execFileSync } from 'node:child_process'
import { WebSocket } from 'ws'
import { LOCAL_TOKEN_FILENAME, type RpcResponse } from '@vornrun/shared/protocol'

/**
 * Where the running server keeps its port and credential files.
 *
 * Both live in the server's data directory, which `vorn-server serve --data-dir`
 * can move. Hard-coding `~/.vorn` meant that a server started anywhere else was
 * invisible here: not just unauthenticated, but undiscoverable, since the port file
 * moves with it. `VORN_DATA_DIR` is how that server tells us where it went.
 */
const DATA_DIR = process.env.VORN_DATA_DIR || path.join(os.homedir(), '.vorn')
const PORT_FILE = path.join(DATA_DIR, 'ws-port')
const LOCAL_TOKEN_FILE = path.join(DATA_DIR, LOCAL_TOKEN_FILENAME)

const TOKEN_FILE_MISSING_MSG = `Vorn local credential not found (${LOCAL_TOKEN_FILE}).
The server writes it on startup and removes it on shutdown, so this usually means
Vorn is not running. Start Vorn (or \`vorn-server serve\`) and try again.
If the server runs with --data-dir, set VORN_DATA_DIR to the same directory.`

/**
 * The running server's local credential.
 *
 * Read on every call rather than cached: it is regenerated each time the server
 * starts, so a cached value would go stale exactly when Vorn is restarted — the
 * moment MCP is most likely to be mid-session.
 */
function readLocalToken(): string {
  try {
    const token = fs.readFileSync(LOCAL_TOKEN_FILE, 'utf-8').trim()
    if (!token) throw new Error('empty')
    return token
  } catch {
    throw new Error(TOKEN_FILE_MISSING_MSG)
  }
}

/**
 * Where to connect and how to prove it, resolved together — both halves fail
 * with their own guidance, and neither is useful without the other.
 *
 * The credential is presented on the upgrade so the socket is authenticated
 * before its first frame: every call here opens a fresh connection, so a
 * handshake round-trip would be paid on all of them.
 */
function connection(): { url: string; options: { headers: Record<string, string> } } {
  const result = readPort()
  if (!result.port) {
    // Narrowed through the union rather than reaching for `.reason` directly,
    // which only exists on the failure arm.
    const reason = 'reason' in result ? result.reason : 'missing'
    throw new Error(reason === 'invalid' ? PORT_FILE_INVALID_MSG : PORT_FILE_MISSING_MSG)
  }
  return {
    url: `ws://127.0.0.1:${result.port}/ws`,
    options: { headers: { Authorization: `Bearer ${readLocalToken()}` } }
  }
}
const TIMEOUT_MS = 10_000

const IS_WIN = process.platform === 'win32'

const PORT_FILE_MISSING_MSG = IS_WIN
  ? `Vorn port file not found (~/.vorn/ws-port).
The app may be running but the port file was deleted (e.g. by another instance shutting down).
To fix, find the Vorn process and its listening port:
  powershell -c "Get-NetTCPConnection -State Listen -OwningProcess (Get-Process Vorn).Id | Select LocalPort"
Then write the WS port to the file:
  echo {"port":<PORT>,"pid":<PID>} > %USERPROFILE%\\.vorn\\ws-port
Or restart Vorn to regenerate it.`
  : `Vorn port file not found (~/.vorn/ws-port).
The app may be running but the port file was deleted (e.g. by another instance shutting down).
To fix, run:  lsof -iTCP -sTCP:LISTEN -P | grep Vorn
Then write the WS port (the one on *:<port>) to the file:
  echo '{"port":<PORT>,"pid":<PID>}' > ~/.vorn/ws-port
Or restart Vorn to regenerate it.`

const PORT_FILE_INVALID_MSG = `Vorn port file exists but contains invalid data (~/.vorn/ws-port).
Delete it and restart Vorn, or overwrite it with the correct port:
  ${IS_WIN ? 'del %USERPROFILE%\\.vorn\\ws-port' : 'rm ~/.vorn/ws-port'}`

let rpcId = 0

// Cache discovered port to avoid repeated execFileSync calls
let cachedPort: number | null = null
let cacheTimestamp = 0
const CACHE_TTL_MS = 5_000

// `stdio` is a plain array rather than `as const`: the readonly tuple that
// produced does not satisfy execFileSync's mutable `StdioOptions`, so every call
// site using these options failed to typecheck.
const EXEC_OPTS = {
  encoding: 'utf-8' as const,
  timeout: 5000,
  stdio: ['pipe', 'pipe', 'pipe'] as ['pipe', 'pipe', 'pipe']
}

/**
 * Try to discover the Vorn WS port by querying the OS for listening sockets.
 * Returns the port number if found, null otherwise.
 */
function discoverPort(): number | null {
  try {
    if (IS_WIN) {
      const taskOut = execFileSync(
        'tasklist',
        ['/FI', 'IMAGENAME eq Vorn.exe', '/FO', 'CSV', '/NH'],
        EXEC_OPTS
      )
      const pidMatch = taskOut.match(/"Vorn\.exe","(\d+)"/)
      if (!pidMatch) return null
      const pid = pidMatch[1]

      const lines = execFileSync('netstat', ['-ano'], EXEC_OPTS).split('\n')
      let fallback: number | null = null
      for (const line of lines) {
        if (!line.includes('LISTENING') || !line.trim().endsWith(pid)) continue
        const m = line.match(/(?:0\.0\.0\.0|127\.0\.0\.1):(\d+)/)
        if (!m) continue
        if (line.includes('0.0.0.0')) return parseInt(m[1], 10)
        fallback ??= parseInt(m[1], 10)
      }
      return fallback
    } else {
      const lines = execFileSync('lsof', ['-iTCP', '-sTCP:LISTEN', '-P', '-n'], EXEC_OPTS).split(
        '\n'
      )
      let fallback: number | null = null
      for (const line of lines) {
        if (!line.includes('Vorn')) continue
        if (line.includes('*:')) {
          const m = line.match(/\*:(\d+)/)
          if (m) return parseInt(m[1], 10)
        }
        if (!fallback) {
          const m = line.match(/:(\d+)\s/)
          if (m) fallback = parseInt(m[1], 10)
        }
      }
      return fallback
    }
  } catch {
    // Command failed or not available
  }
  return null
}

/** Try OS-level discovery, cache result, and heal the port file. */
function discoverAndHeal(): { port: number } | { port: null; reason: 'missing' } {
  const now = Date.now()
  if (cachedPort && now - cacheTimestamp < CACHE_TTL_MS) return { port: cachedPort }

  const discovered = discoverPort()
  cachedPort = discovered
  cacheTimestamp = now
  if (discovered) {
    try {
      fs.mkdirSync(path.dirname(PORT_FILE), { recursive: true })
      fs.writeFileSync(PORT_FILE, JSON.stringify({ port: discovered }), 'utf-8')
    } catch {
      // best-effort
    }
    return { port: discovered }
  }
  return { port: null, reason: 'missing' }
}

/**
 * Read the Vorn server port from the well-known file.
 * Falls back to OS-level port discovery if the file is missing or stale.
 */
function readPort(): { port: number } | { port: null; reason: 'missing' | 'invalid' } {
  try {
    const raw = fs.readFileSync(PORT_FILE, 'utf-8').trim()
    if (!raw) return { port: null, reason: 'invalid' }

    // JSON format: { "port": 53829, "pid": 1234 }
    if (raw.startsWith('{')) {
      const parsed = JSON.parse(raw)
      const p = parsed?.port
      const pid = parsed?.pid
      if (typeof p !== 'number' || !Number.isFinite(p) || p <= 0) {
        return { port: null, reason: 'invalid' }
      }
      // If PID is present, verify the process is still alive
      if (typeof pid === 'number' && Number.isInteger(pid) && pid > 0) {
        try {
          process.kill(pid, 0)
        } catch (err: unknown) {
          // EPERM means the process exists but we lack permission — treat as alive
          if ((err as NodeJS.ErrnoException).code === 'EPERM') return { port: p }
          // PID is dead — port file is stale, fall through to discovery
          return discoverAndHeal()
        }
      }
      return { port: p }
    }

    // Legacy plain-number format: 53829
    const p = parseInt(raw, 10)
    return Number.isFinite(p) && p > 0 ? { port: p } : { port: null, reason: 'invalid' }
  } catch {
    // Port file missing — try OS-level discovery
    return discoverAndHeal()
  }
}

/**
 * Send a single JSON-RPC request to the running Vorn server over WebSocket.
 * Opens a connection, sends, waits for the response, then closes.
 */
export async function rpcCall<T = unknown>(
  method: string,
  params?: unknown,
  /**
   * Overrides the default ceiling for the few calls that legitimately take
   * longer — starting a connector package downloads it first, which the
   * default would abort while it was still working.
   */
  timeoutMs: number = TIMEOUT_MS
): Promise<T> {
  const { url, options } = connection()

  return new Promise<T>((resolve, reject) => {
    const ws = new WebSocket(url, options)
    const id = ++rpcId

    const timer = setTimeout(() => {
      ws.close()
      reject(new Error(`RPC call "${method}" timed out after ${timeoutMs}ms`))
    }, timeoutMs)

    ws.on('open', () => {
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })

    ws.on('message', (raw: Buffer) => {
      try {
        const msg: RpcResponse = JSON.parse(raw.toString())
        if (msg.id !== id) return // ignore broadcasts / notifications
        clearTimeout(timer)
        ws.close()
        if (msg.error) {
          reject(new Error(msg.error.message))
        } else {
          resolve(msg.result as T)
        }
      } catch {
        // ignore non-JSON messages
      }
    })

    ws.on('error', (err) => {
      clearTimeout(timer)
      reject(new Error(`Cannot connect to Vorn server: ${err.message}. Is the app running?`))
    })
  })
}

/**
 * Send a fire-and-forget JSON-RPC notification (no response expected).
 */
export async function rpcNotify(method: string, params?: unknown): Promise<void> {
  const { url, options } = connection()

  return new Promise<void>((resolve, reject) => {
    const ws = new WebSocket(url, options)

    ws.on('open', () => {
      // No id = fire-and-forget notification per JSON-RPC spec
      ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
      ws.close()
      resolve()
    })

    ws.on('error', (err) => {
      reject(new Error(`Cannot connect to Vorn server: ${err.message}. Is the app running?`))
    })
  })
}

/**
 * Check whether the Vorn server is reachable.
 */
export function isServerRunning(): boolean {
  return readPort().port !== null
}
