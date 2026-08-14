import { spawn, type ChildProcess } from 'node:child_process'
import { createInterface } from 'node:readline'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import * as grpc from '@grpc/grpc-js'
import * as protoLoader from '@grpc/proto-loader'
import { app } from 'electron'
import log from './logger'

/**
 * One `idb_companion` child process per claimed simulator, and the gRPC client
 * that talks to it.
 *
 * A simulator has no `<webview>` equivalent: there is no in-process guest to
 * attach a debugger to, so main owns a real child process and speaks gRPC to it
 * over a unix socket. That makes the lifetime questions — who started it, who
 * kills it, what happens when it dies on its own — this module's whole job.
 *
 * Modelled on `server/server-launcher.ts`, which is the only other place in main
 * that owns a child process: spawn, read a JSON line off stdout to learn where
 * to connect, and tear down on quit.
 */

/** The subset of `idb.CompanionService` we call. Untyped by design: the client
 *  is built from the proto at runtime, so there are no generated stubs. */
export interface CompanionClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [method: string]: any
}

export interface CompanionHandle {
  udid: string
  child: ChildProcess
  client: CompanionClient
  socketPath: string
}

/** Every companion this process started, so `before-quit` can kill all of them.
 *  A stranded companion holds both a socket file and a live simulator. */
const running = new Map<string, CompanionHandle>()

/**
 * Where the vendored proto lives, in dev and packaged.
 *
 * `idb_companion` serves no reflection, so the schema cannot be discovered at
 * runtime — the file has to be on disk either way. Mirrors
 * `resolveServerEntry`'s dev/packaged branch.
 */
export function resolveProtoPath(): string {
  if (process.env.ELECTRON_RENDERER_URL) {
    return path.join(__dirname, '../../resources/idb.proto')
  }
  return path.join(process.resourcesPath, 'idb.proto')
}

/**
 * The unix socket for a device's companion.
 *
 * A socket rather than a TCP port so two sessions driving two simulators cannot
 * collide on a port number, and so nothing is listening on the network. The
 * path lives under a short temp dir because sun_path is capped near 104 bytes
 * on macOS — `app.getPath('userData')` is long enough to overflow it for some
 * users, and the failure is an opaque bind error.
 */
export function socketPathFor(udid: string, dir: string = os.tmpdir()): string {
  return path.join(dir, `vorn-idb-${udid.slice(0, 8).toLowerCase()}.sock`)
}

/** The one companion binary we shell out to, resolved for the error taxonomy. */
const COMPANION_BIN = 'idb_companion'

/**
 * Reads the readiness line the companion writes on startup, so we know it is
 * actually listening before the first call.
 *
 * In domain-socket mode that line is `{"grpc_path":"/tmp/…sock"}` (verified
 * against companion 1.1.8); the port keys are the TCP-mode spellings, accepted
 * so a future `--grpc-port` path does not silently hang here.
 *
 * Without this, the first RPC races the bind and fails with a connection error
 * that reads like "the simulator is broken" rather than "not up yet".
 */
export function readCompanionReady(child: ChildProcess, timeoutMs = 30_000): Promise<void> {
  return new Promise((resolve, reject) => {
    if (!child.stdout) {
      reject(new Error('No stdout on idb_companion process'))
      return
    }
    let settled = false
    const done = (err?: Error): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      rl.close()
      if (err) reject(err)
      else resolve()
    }

    const rl = createInterface({ input: child.stdout })
    const timer = setTimeout(
      () => done(new Error('Timed out waiting for idb_companion to start.')),
      timeoutMs
    )

    rl.on('line', (line) => {
      try {
        const parsed = JSON.parse(line) as Record<string, unknown>
        // The key has changed across companion versions; any of these means
        // the server is bound and listening.
        if ('grpc_swift_port' in parsed || 'grpc_port' in parsed || 'grpc_path' in parsed) {
          done()
        }
      } catch {
        // Companion logs plain text on stdout too — not every line is JSON.
      }
    })

    child.on('exit', (code) => {
      done(new Error(`idb_companion exited before it was ready (code=${code}).`))
    })
    child.on('error', (err) => {
      done(
        (err as NodeJS.ErrnoException).code === 'ENOENT'
          ? new Error(
              `${COMPANION_BIN} is not installed. Install it with:\n  brew install facebook/fb/idb-companion`
            )
          : err
      )
    })
  })
}

/** Builds the gRPC client from the vendored proto. */
function connect(socketPath: string): CompanionClient {
  const def = protoLoader.loadSync(resolveProtoPath(), {
    keepCase: true,
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true
  })
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const pkg = grpc.loadPackageDefinition(def) as any
  return new pkg.idb.CompanionService(`unix://${socketPath}`, grpc.credentials.createInsecure(), {
    // A full-resolution screenshot is ~3 MB; the 4 MB default leaves no
    // headroom on a larger device and fails as a confusing RESOURCE_EXHAUSTED.
    'grpc.max_receive_message_length': 32 * 1024 * 1024
  })
}

/**
 * Starts a companion for `udid` and returns a connected client.
 *
 * `onExit` is the analogue of the browser registry's debugger-detach handler:
 * when the companion dies — the simulator was shut down from Simulator.app, the
 * process was killed, the machine slept — the registry has to hear about it and
 * mark the entry unattached. Otherwise the next tool call hangs on a dead
 * socket instead of saying the connection dropped.
 */
export async function startCompanion(
  udid: string,
  onExit: (udid: string, handle: CompanionHandle) => void
): Promise<CompanionHandle> {
  const existing = running.get(udid)
  if (existing) return existing

  const socketPath = socketPathFor(udid)
  // A leftover socket file from a crashed companion makes bind fail.
  try {
    fs.unlinkSync(socketPath)
  } catch {
    // Nothing there, which is the normal case.
  }

  const child = spawn(
    COMPANION_BIN,
    [
      '--udid',
      udid,
      '--grpc-domain-sock',
      socketPath,
      // A simulator that vanishes should take its companion with it rather
      // than leaving a process holding a socket for a device that is gone.
      '--terminate-offline',
      '1'
    ],
    { stdio: ['ignore', 'pipe', 'pipe'] }
  )

  if (child.stderr) {
    const errLines = createInterface({ input: child.stderr })
    errLines.on('line', (line) => log.info(`[idb ${udid.slice(0, 8)}] ${line}`))
  }

  try {
    await readCompanionReady(child)
  } catch (err) {
    child.kill('SIGKILL')
    throw err
  }

  const handle: CompanionHandle = { udid, child, client: connect(socketPath), socketPath }
  running.set(udid, handle)

  child.on('exit', (code, signal) => {
    log.warn(`[idb ${udid.slice(0, 8)}] companion exited (code=${code}, signal=${signal})`)
    // Identity, not udid. `stopCompanion` drops the handle and sends SIGTERM,
    // but the child can take seconds to actually die — and a person who closes
    // a device pane and immediately reopens the same device has by then spawned
    // a *replacement* companion under this same udid. Deleting and reporting by
    // udid alone would let the corpse's exit tear down its live successor: the
    // new pane would sit on "connection dropped" forever while a perfectly
    // healthy companion ran on, orphaned from both the map and the entry.
    if (running.get(udid) === handle) {
      running.delete(udid)
      try {
        // Only ours to remove, for the same reason: a replacement companion
        // binds this exact path, and unlinking it out from under one leaves a
        // live process nobody can connect to.
        fs.unlinkSync(socketPath)
      } catch {
        // Best effort; the socket may already be gone.
      }
    }
    onExit(udid, handle)
  })

  return handle
}

/** Kills the companion for one device. Safe to call when none is running. */
export function stopCompanion(udid: string): void {
  const handle = running.get(udid)
  if (!handle) return
  running.delete(udid)
  try {
    handle.client.close?.()
  } catch {
    // Closing a client whose channel already failed throws; not worth caring.
  }
  const child = handle.child
  let exited = false
  child.once('exit', () => {
    exited = true
  })
  child.kill('SIGTERM')
  // `child.killed` only records that a signal was delivered, not that the
  // process died — checking it would skip SIGKILL for exactly the companion
  // that ignored SIGTERM, which is the one case this fallback exists for.
  setTimeout(() => {
    if (!exited) child.kill('SIGKILL')
  }, 3000)
}

export function stopAllCompanions(): void {
  for (const udid of [...running.keys()]) stopCompanion(udid)
}

export function runningCompanion(udid: string): CompanionHandle | undefined {
  return running.get(udid)
}

let quitHookInstalled = false
/** Registered once from main's setup; a stranded companion outlives the app. */
export function installCompanionQuitHook(): void {
  if (quitHookInstalled) return
  quitHookInstalled = true
  app.on('before-quit', stopAllCompanions)
}

/**
 * Wraps a unary companion call in a promise.
 *
 * gRPC-js is callback-first, and a status error carries the detail that makes
 * the failure actionable — dropping it for a bare "call failed" costs the agent
 * a whole turn guessing.
 *
 * Two shapes of the response are worth knowing before you build on this, both
 * verified against companion 1.1.8 rather than read off the proto:
 *  - `accessibility_info` returns `json` holding an **array** whose first entry
 *    is the application root, not a bare object.
 *  - `screenshot` returns `image_format` as an empty string, so the format has
 *    to be sniffed from the bytes (they are PNG) rather than trusted.
 */
export function call<T>(client: CompanionClient, method: string, request: unknown): Promise<T> {
  return new Promise((resolve, reject) => {
    client[method](request, (err: grpc.ServiceError | null, res: T) => {
      if (err) reject(new Error(err.details || err.message))
      else resolve(res)
    })
  })
}

/**
 * How long any streaming call may take before we give up on it.
 *
 * Without this a call that never answers hangs its caller for the life of the
 * process, and an agent waiting on a tool result has no way to tell that apart
 * from a slow simulator. Generous enough that a cold app launch fits.
 */
const STREAM_TIMEOUT_MS = 30_000

/**
 * Writes a sequence of messages to a **client-streaming** call — `stream X → Y`
 * — and waits for the single reply.
 *
 * `hid` is the one that matters: a tap is *two* events, a DOWN and an UP, and
 * sending only the first leaves a finger held on the glass — every subsequent
 * gesture then behaves strangely for reasons nothing reports. Keeping both
 * halves inside one call is what makes that impossible to get half-right at a
 * call site.
 *
 * Only for methods the proto declares as `stream X` returning a bare `Y`. A
 * method that streams *back* has no callback form at all, and passing one there
 * is a hang rather than an error — see `callBidiStreaming`.
 */
export function callStreaming<T>(
  client: CompanionClient,
  method: string,
  messages: readonly unknown[]
): Promise<T> {
  return new Promise((resolve, reject) => {
    let settled = false
    // The timer is armed before the call is opened, so a client that answers
    // synchronously — a mock, or a future grpc-js fast path — finds it already
    // there. Held in an object because cancelling needs the stream, and the
    // stream does not exist until after the timer.
    const open: { stream?: grpc.ClientWritableStream<unknown> } = {}
    const timer = setTimeout(() => {
      if (settled) return
      settled = true
      open.stream?.cancel()
      reject(new Error(`The device did not answer ${method} within ${STREAM_TIMEOUT_MS / 1000}s.`))
    }, STREAM_TIMEOUT_MS)
    const settle = (fn: () => void): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      fn()
    }

    const stream = client[method]((err: grpc.ServiceError | null, res: T) => {
      if (err) settle(() => reject(new Error(err.details || err.message)))
      else settle(() => resolve(res))
    })
    open.stream = stream

    for (const m of messages) stream.write(m)
    stream.end()
  })
}

export function callBidiStreaming<T>(
  client: CompanionClient,
  method: string,
  messages: readonly unknown[]
): Promise<T> {
  return new Promise((resolve, reject) => {
    const stream = client[method]() as grpc.ClientDuplexStream<unknown, T>
    let last: T | undefined
    let settled = false
    const settle = (fn: () => void): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      fn()
    }
    const timer = setTimeout(() => {
      if (settled) return
      settled = true
      stream.cancel()
      reject(new Error(`The device did not answer ${method} within ${STREAM_TIMEOUT_MS / 1000}s.`))
    }, STREAM_TIMEOUT_MS)

    stream.on('data', (msg: T) => {
      last = msg
    })
    stream.on('error', (err: grpc.ServiceError) => {
      settle(() => reject(new Error(err.details || err.message)))
    })
    stream.on('end', () => {
      settle(() => resolve((last ?? {}) as T))
    })

    for (const m of messages) stream.write(m)
    stream.end()
  })
}
