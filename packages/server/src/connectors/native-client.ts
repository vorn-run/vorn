// A connector child that speaks Vorn's native protocol: one JSON-RPC message per line on stdin and stdout.
import crossSpawn from 'cross-spawn'
import {
  MAX_FRAME_BYTES,
  PROTOCOL_ERROR_CODES,
  PROTOCOL_ERROR_KINDS,
  type ProtocolErrorKind,
  type ProtocolMethod,
  type ProtocolMethods
} from '@vornrun/shared/connector-protocol'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

export interface NativeSpawnConfig {
  command: string
  args: string[]
  env: Record<string, string>
  cwd?: string
}

export interface NativeClientOptions {
  label: string
  key: string
  maxFrameBytes?: number
  stderrTailLines?: number
  closeTimings?: { termAfterMs: number; killAfterMs: number }
}

export interface ChildExit {
  code: number | null
  signal: string | null
}

/** A request the child answered with an error. */
export class SdkCallError extends Error {
  readonly method: string
  readonly code: number
  readonly kind?: ProtocolErrorKind
  readonly retryable?: boolean
  readonly field?: string
  readonly output?: Record<string, unknown>

  constructor(
    method: string,
    code: number,
    message: string,
    details: {
      kind?: ProtocolErrorKind
      retryable?: boolean
      field?: string
      output?: Record<string, unknown>
    } = {}
  ) {
    super(message)
    this.name = 'SdkCallError'
    this.method = method
    this.code = code
    if (details.kind !== undefined) this.kind = details.kind
    if (details.retryable !== undefined) this.retryable = details.retryable
    if (details.field !== undefined) this.field = details.field
    if (details.output !== undefined) this.output = details.output
  }
}

export type TransportFailure = 'spawn' | 'exited' | 'timeout' | 'oversized' | 'closed'

/** A request that got no answer because the child or its pipe failed. */
export class SdkTransportError extends Error {
  readonly reason: TransportFailure
  readonly stderrTail: string[]
  readonly exitCode?: number | null
  readonly signal?: string | null

  constructor(
    reason: TransportFailure,
    message: string,
    details: { stderrTail?: string[]; exit?: ChildExit } = {}
  ) {
    super(message)
    this.name = 'SdkTransportError'
    this.reason = reason
    this.stderrTail = details.stderrTail ?? []
    if (details.exit) {
      this.exitCode = details.exit.code
      this.signal = details.exit.signal
    }
  }
}

export interface NativeClient {
  request<M extends ProtocolMethod>(
    method: M,
    params: ProtocolMethods[M]['params'],
    timeoutMs: number
  ): Promise<ProtocolMethods[M]['result']>
  /** Ends stdin, then SIGTERM and SIGKILL if the child lingers; safe to call twice. */
  close(): Promise<void>
  onExit(listener: (exit: ChildExit) => void): void
  readonly exited: boolean
  readonly pid: number | undefined
  stderrTail(): string[]
}

const DEFAULT_TAIL_LINES = 40
const MAX_STDERR_LINE = 4096
const DEFAULT_CLOSE = { termAfterMs: 2_000, killAfterMs: 5_000 }

/** Split a byte stream into lines, decoding only whole lines so a character split across chunks survives. */
export function createFrameReader(opts: {
  maxBytes: number
  onLine(line: string): void
  onOversize(): void
}): (chunk: Buffer) => void {
  let parts: Buffer[] = []
  let pending = 0
  let dead = false
  const oversize = (): void => {
    dead = true
    parts = []
    opts.onOversize()
  }
  return (chunk) => {
    if (dead) return
    let start = 0
    for (;;) {
      const end = chunk.indexOf(0x0a, start)
      if (end === -1) break
      const piece = chunk.subarray(start, end)
      if (pending + piece.length > opts.maxBytes) return oversize()
      const bytes = pending === 0 ? piece : Buffer.concat([...parts, piece])
      parts = []
      pending = 0
      const line = bytes.toString('utf8').replace(/\r$/, '')
      if (line.trim() !== '') opts.onLine(line)
      start = end + 1
    }
    if (start < chunk.length) {
      const rest = chunk.subarray(start)
      pending += rest.length
      if (pending > opts.maxBytes) return oversize()
      parts.push(rest)
    }
  }
}

// A copy of errorLine in packages/connector-sdk/src/packaging.ts, which the server does not import.
export function errorLine(lines: readonly string[]): string | undefined {
  const kept = lines.map((line) => line.trim()).filter((line) => line !== '')
  return [...kept].reverse().find((line) => /Error\b/.test(line)) ?? kept[kept.length - 1]
}

function describeExit(exit: ChildExit): string {
  return exit.signal ? `signal ${exit.signal}` : `code ${exit.code}`
}

function callError(method: string, raw: unknown): SdkCallError {
  const error = (raw && typeof raw === 'object' ? raw : {}) as {
    code?: unknown
    message?: unknown
    data?: unknown
  }
  const data = (error.data && typeof error.data === 'object' ? error.data : {}) as {
    kind?: unknown
    retryable?: unknown
    field?: unknown
  }
  const kind = (PROTOCOL_ERROR_KINDS as readonly unknown[]).includes(data.kind)
    ? (data.kind as ProtocolErrorKind)
    : undefined
  return new SdkCallError(
    method,
    typeof error.code === 'number' ? error.code : PROTOCOL_ERROR_CODES.connectorError,
    typeof error.message === 'string' && error.message !== '' ? error.message : `${method} failed`,
    {
      ...(kind && { kind }),
      ...(typeof data.retryable === 'boolean' && { retryable: data.retryable }),
      ...(typeof data.field === 'string' && { field: data.field })
    }
  )
}

interface Pending {
  method: string
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: NodeJS.Timeout
}

export async function startNativeClient(
  config: NativeSpawnConfig,
  options: NativeClientOptions
): Promise<NativeClient> {
  const maxBytes = options.maxFrameBytes ?? MAX_FRAME_BYTES
  const tailLines = options.stderrTailLines ?? DEFAULT_TAIL_LINES
  const timings = options.closeTimings ?? DEFAULT_CLOSE
  const name = `[${options.label}] ${options.key}`

  const child = crossSpawn(config.command, config.args, {
    ...(config.cwd !== undefined && { cwd: config.cwd }),
    // The same sanitized base every child gets; what a caller names still wins.
    env: { ...getSafeEnv(), ...config.env },
    stdio: ['pipe', 'pipe', 'pipe'],
    shell: false,
    windowsHide: true
  })
  const { stdin, stdout, stderr } = child
  if (!stdin || !stdout || !stderr) {
    throw new SdkTransportError('spawn', `${name} started ${config.command} without its pipes`)
  }

  const tail: string[] = []
  const pending = new Map<number, Pending>()
  const exitListeners: Array<(exit: ChildExit) => void> = []
  let nextId = 1
  let spawned = false
  let exited = false
  let closing = false
  let exit: ChildExit | undefined
  let closed: Promise<void> | undefined
  let stderrRest = ''

  const tailCopy = (): string[] => [...tail]
  const withLine = (message: string): string => {
    const line = errorLine(tail)
    return line && !message.includes(line) ? `${message}: ${line}` : message
  }
  const rejectAll = (error: (method: string) => Error): void => {
    for (const [id, entry] of pending) {
      clearTimeout(entry.timer)
      pending.delete(id)
      entry.reject(error(entry.method))
    }
  }

  const keepStderr = (line: string): void => {
    const kept = line.length > MAX_STDERR_LINE ? line.slice(0, MAX_STDERR_LINE) : line
    log.info(`${name} stderr: ${kept}`)
    tail.push(kept)
    if (tail.length > tailLines) tail.shift()
  }
  stderr.on('data', (chunk: Buffer) => {
    stderrRest += chunk.toString('utf8')
    const lines = stderrRest.split('\n')
    stderrRest = lines.pop() ?? ''
    for (const line of lines) {
      const text = line.replace(/\r$/, '')
      if (text.trim() !== '') keepStderr(text)
    }
    if (stderrRest.length > MAX_STDERR_LINE) {
      keepStderr(stderrRest)
      stderrRest = ''
    }
  })

  const onLine = (line: string): void => {
    let message: unknown
    try {
      message = JSON.parse(line)
    } catch {
      log.warn(`${name} wrote a line that is not JSON: ${line.slice(0, 200)}`)
      return
    }
    if (!message || typeof message !== 'object' || Array.isArray(message)) {
      log.warn(`${name} wrote a line that is not a message: ${line.slice(0, 200)}`)
      return
    }
    const frame = message as { id?: unknown; result?: unknown; error?: unknown; method?: unknown }
    if (typeof frame.method === 'string') {
      log.warn(`${name} sent ${frame.method}, which a connector does not send in protocol 1`)
      return
    }
    const entry = typeof frame.id === 'number' ? pending.get(frame.id) : undefined
    if (!entry || typeof frame.id !== 'number') {
      log.warn(`${name} answered a request nobody is waiting for: ${String(frame.id)}`)
      return
    }
    pending.delete(frame.id)
    clearTimeout(entry.timer)
    if (frame.error !== undefined) entry.reject(callError(entry.method, frame.error))
    else entry.resolve(frame.result)
  }

  stdout.on(
    'data',
    createFrameReader({
      maxBytes,
      onLine,
      onOversize: () => {
        log.warn(`${name} wrote a line over ${maxBytes} bytes; stopping it`)
        rejectAll(
          (method) =>
            new SdkTransportError(
              'oversized',
              `${name} answered ${method} with a line over ${maxBytes} bytes`,
              {
                stderrTail: tailCopy()
              }
            )
        )
        child.kill('SIGKILL')
      }
    })
  )
  // A child that dies mid-write must not take the server with it.
  stdin.on('error', (err) => log.warn(`${name} stdin: ${err.message}`))
  stdout.on('error', (err) => log.warn(`${name} stdout: ${err.message}`))

  child.on('close', (code, signal) => {
    if (stderrRest.trim() !== '') keepStderr(stderrRest)
    stderrRest = ''
    exited = true
    exit = { code, signal }
    const ended = exit
    rejectAll((method) =>
      closing
        ? new SdkTransportError('closed', `${name} was stopped before it answered ${method}`, {
            stderrTail: tailCopy(),
            exit: ended
          })
        : new SdkTransportError(
            'exited',
            withLine(`${name} exited (${describeExit(ended)}) before it answered ${method}`),
            { stderrTail: tailCopy(), exit: ended }
          )
    )
    if (!closing) log.info(`${name} exited (${describeExit(ended)})`)
    for (const listener of exitListeners) listener(ended)
  })

  const client: NativeClient = {
    request(method, params, timeoutMs) {
      if (exited || closing) {
        return Promise.reject(
          new SdkTransportError(
            closing ? 'closed' : 'exited',
            withLine(`${name} is not running, so it cannot answer ${method}`),
            { stderrTail: tailCopy(), ...(exit && { exit }) }
          )
        )
      }
      const id = nextId++
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          if (!pending.delete(id)) return
          reject(
            new SdkTransportError(
              'timeout',
              `${name} did not answer ${method} within ${Math.round(timeoutMs / 1000)} s`,
              { stderrTail: tailCopy() }
            )
          )
        }, timeoutMs)
        pending.set(id, {
          method,
          resolve: resolve as (value: unknown) => void,
          reject,
          timer
        })
        stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`)
      })
    },
    close() {
      if (closed) return closed
      closing = true
      rejectAll(
        (method) =>
          new SdkTransportError('closed', `${name} was stopped before it answered ${method}`, {
            stderrTail: tailCopy()
          })
      )
      closed = new Promise<void>((resolve) => {
        if (exited || !spawned) {
          if (!exited) child.kill('SIGKILL')
          resolve()
          return
        }
        const term = setTimeout(() => {
          if (!exited) child.kill('SIGTERM')
        }, timings.termAfterMs)
        const kill = setTimeout(() => {
          if (!exited) child.kill('SIGKILL')
        }, timings.killAfterMs)
        // A child whose pipes outlive it must not leave close() waiting for ever.
        const giveUp = setTimeout(resolve, timings.killAfterMs + 1_000)
        term.unref()
        kill.unref()
        giveUp.unref()
        child.once('close', () => {
          clearTimeout(term)
          clearTimeout(kill)
          clearTimeout(giveUp)
          resolve()
        })
        stdin.end()
      })
      return closed
    },
    onExit(listener) {
      if (exit) {
        const ended = exit
        queueMicrotask(() => listener(ended))
      } else exitListeners.push(listener)
    },
    get exited() {
      return exited
    },
    get pid() {
      return child.pid
    },
    stderrTail: tailCopy
  }

  return new Promise<NativeClient>((resolve, reject) => {
    child.once('error', (err) => {
      if (spawned) {
        log.warn(`${name}: ${err.message}`)
        return
      }
      exited = true
      reject(
        new SdkTransportError(
          'spawn',
          `${name} could not start ${config.command}: ${err.message}`,
          {
            stderrTail: tailCopy()
          }
        )
      )
    })
    child.once('spawn', () => {
      spawned = true
      resolve(client)
    })
  })
}
