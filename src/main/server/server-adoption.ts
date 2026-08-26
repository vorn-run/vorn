import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import {
  LOCAL_TOKEN_FILENAME,
  WS_PORT_FILENAME,
  RUNTIME_PROTOCOL_VERSION,
  type ServerHello
} from '@vornrun/shared/protocol'

/**
 * Deciding whether a server that is already running is ours to use.
 *
 * The question only exists because the server now outlives the app. While it was
 * a child, "is one running" had one answer per launch; detached, an updater
 * restart, a crash respawn, a second launch and a `yarn dev` beside a packaged
 * build all reach this code against a live incumbent.
 *
 * The rule this file exists to hold, taken from the terminal multiplexers that
 * have lived with it longest: **the incumbent wins.** tmux answers a bad protocol
 * version by marking the peer bad and letting the *client* exit; zellij keeps the
 * wire contract in its socket directory name so mismatched pairs never meet. In
 * neither does a starting process end a running one. The server holding the PTYs
 * is the one with the user's work in it, so a client that cannot speak to it
 * declines and says so — it never resolves the disagreement by killing.
 */

/**
 * The data directory the server this app spawns will use.
 *
 * Deliberately NOT `VORN_DATA_DIR`, even though `packages/mcp` reads that. The
 * launcher never passes `--data-dir`, so its server always resolves `~/.vorn` --
 * and the server injects `VORN_DATA_DIR` into everything it launches, so a Vorn
 * started from a terminal *inside* a Vorn session inherits a value describing
 * somebody else's data directory. Honouring it here would send the launcher
 * looking for a port file that its own server is never going to write.
 */
export function resolveDataDir(): string {
  return path.join(os.homedir(), '.vorn')
}

export type PortFile = { port: number; pid?: number }

/**
 * The `{port, pid}` a running server publishes, or null.
 *
 * A stale file is the normal case after a SIGKILL — the server deletes it on an
 * orderly exit only. So a record whose pid is dead is treated as absent rather
 * than trusted; the port it names may since have been taken by anything.
 */
export function readPortFile(dataDir = resolveDataDir()): PortFile | null {
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(dataDir, WS_PORT_FILENAME), 'utf-8'))
    const port = typeof raw?.port === 'number' ? raw.port : null
    if (port === null) return null
    const pid = typeof raw?.pid === 'number' ? raw.pid : undefined
    if (pid !== undefined && !isPidAlive(pid)) return null
    return pid === undefined ? { port } : { port, pid }
  } catch {
    return null
  }
}

/**
 * Whether a pid is running.
 *
 * `EPERM` means alive and owned by somebody else, which is still alive — the
 * mistake to avoid is collapsing "cannot tell" into "dead", because acting on
 * that answer is how a healthy server gets replaced.
 */
export function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === 'EPERM'
  }
}

/** The credential a server publishes for same-machine callers, or null. */
export function readLocalToken(dataDir = resolveDataDir()): string | null {
  try {
    const token = fs.readFileSync(path.join(dataDir, LOCAL_TOKEN_FILENAME), 'utf-8').trim()
    return token.length > 0 ? token : null
  } catch {
    return null
  }
}

export type AdoptionVerdict =
  | { kind: 'adopt' }
  | { kind: 'refuse'; reason: RefusalReason; detail: string }

export type RefusalReason =
  | 'no-identity'
  | 'protocol-mismatch'
  | 'different-data-dir'
  | 'different-build'

/**
 * Whether the greeting on the other end belongs to a server this app may use.
 *
 * Gated on `protocolVersion`, never on `appVersion`. The two move independently
 * on purpose: the protocol changes when the messages change, the release changes
 * every time anything ships. Gating on the release would end every running
 * session on every update for no reason at all, which is exactly the trap zellij
 * avoids by keeping `CLIENT_SERVER_CONTRACT_VERSION` separate from `VERSION`.
 */
export function judgeAdoption(
  hello: ServerHello | null,
  self: { dataDir: string; buildChannel: 'dev' | 'packaged' }
): AdoptionVerdict {
  if (!hello) {
    return { kind: 'refuse', reason: 'no-identity', detail: 'no greeting arrived' }
  }
  // A server old enough to predate these fields cannot be told apart from one on
  // another data directory, so it is not adoptable. Declining costs a spawn;
  // guessing costs two servers on one database.
  // The pid is required, not decorative: it is the only way to tell a server
  // that died from a bridge that is merely reconnecting, and the only handle
  // left for stopping one this app did not spawn. Adopting without it yields a
  // server that can be neither recovered nor ended.
  if (
    hello.dataDir === undefined ||
    hello.buildChannel === undefined ||
    typeof hello.pid !== 'number'
  ) {
    return {
      kind: 'refuse',
      reason: 'no-identity',
      detail: 'the running server does not report its identity'
    }
  }
  if (hello.protocolVersion !== RUNTIME_PROTOCOL_VERSION) {
    return {
      kind: 'refuse',
      reason: 'protocol-mismatch',
      detail: `it speaks protocol ${hello.protocolVersion}, this app speaks ${RUNTIME_PROTOCOL_VERSION}`
    }
  }
  if (path.resolve(hello.dataDir) !== path.resolve(self.dataDir)) {
    return {
      kind: 'refuse',
      reason: 'different-data-dir',
      detail: `it holds ${hello.dataDir}, this app wants ${self.dataDir}`
    }
  }
  if (hello.buildChannel !== self.buildChannel) {
    return {
      kind: 'refuse',
      reason: 'different-build',
      detail: `it is a ${hello.buildChannel} build, this app is ${self.buildChannel}`
    }
  }
  return { kind: 'adopt' }
}
