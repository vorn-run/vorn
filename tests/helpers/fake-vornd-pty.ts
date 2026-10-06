import { EventEmitter } from 'node:events'
import { vi } from 'vitest'
import type { HeldSession, VorndExit, VorndSpawn } from '../../packages/server/src/vornd-sessions'

/**
 * A stand-in for `vorndSessions`, for tests of what the server does with a
 * session rather than of the channel to vornd. Each spawn returns a
 * `FakeVorndPty` the test drives: it says what vornd would (started, status,
 * cwd, activity, exit, output for a watched session) and records what it was
 * asked to do.
 *
 * Mock the module with it from the test file, where `vi.mock` is hoisted:
 *
 *   vi.mock('../packages/server/src/vornd-sessions', async () =>
 *     (await import('./helpers/fake-vornd-pty')).vorndModule())
 */

let nextPid = 1000

export class FakeVorndPty extends EventEmitter {
  pid = 0
  /** Everything written to the session, in order. */
  written: string[] = []
  /** Each signal it was sent. */
  kills: string[] = []
  stdinClosed = false
  /** Thrown by `kill`, as for a session already gone. */
  killError: Error | null = null
  /** The record cursor of the exit effect, as `VorndPty.exitAt` reports it. */
  exitAt: { epoch: number; rseq: number; index: number } | null = null
  private ended = false
  private readonly dataListeners = new Set<(data: string) => void>()
  private readonly exitListeners = new Set<(event: VorndExit) => void>()

  constructor(
    readonly id: string,
    /** What vornd was asked to start; null for a session taken on. */
    readonly spec: VorndSpawn | null,
    readonly watched: boolean
  ) {
    super()
  }

  write(data: string): void {
    if (this.ended || data.length === 0) return
    this.written.push(data)
  }

  resize(): void {}

  kill(signal = 'SIGHUP'): void {
    if (this.killError) throw this.killError
    if (this.ended) return
    this.kills.push(signal)
  }

  closeStdin(): void {
    this.stdinClosed = true
  }

  onData(listener: (data: string) => void): { dispose(): void } {
    this.dataListeners.add(listener)
    return { dispose: () => this.dataListeners.delete(listener) }
  }

  onExit(listener: (event: VorndExit) => void): { dispose(): void } {
    this.exitListeners.add(listener)
    return { dispose: () => this.exitListeners.delete(listener) }
  }

  get isEnded(): boolean {
    return this.ended
  }

  /** vornd answered the spawn. */
  start(pid = nextPid++): void {
    this.pid = pid
    this.emit('started', pid)
  }

  /** A status effect, by its code in `NATIVE_STATUS`, and the record it came from. */
  status(code: number, at?: { epoch: number; rseq: number; index: number }): void {
    this.emit('status', code, at)
  }

  cwd(dir: string): void {
    this.emit('cwd', dir)
  }

  activity(): void {
    this.emit('activity')
  }

  /** Output read from the session, which only a watched one has. */
  print(data: string): void {
    for (const listener of [...this.dataListeners]) listener(data)
  }

  exit(exitCode = 0, repeated = false): void {
    if (this.ended) return
    this.ended = true
    for (const listener of [...this.exitListeners]) listener({ exitCode, repeated })
  }
}

const spawned: FakeVorndPty[] = []

function spawnFake(id: string, spec: VorndSpawn, watched: boolean): FakeVorndPty {
  const pty = new FakeVorndPty(id, spec, watched)
  spawned.push(pty)
  return pty
}

function adoptFake(
  held: HeldSession,
  watched: boolean,
  wire?: (pty: FakeVorndPty) => void
): FakeVorndPty {
  const pty = new FakeVorndPty(held.id, null, watched)
  wire?.(pty)
  pty.start(held.pid)
  if (held.status) pty.status(held.status.status ?? 0)
  if (held.cwd?.cwd) pty.cwd(held.cwd.cwd)
  return pty
}

export const fakeVornd = {
  spawned,
  spawn: vi.fn(spawnFake),
  adopt: vi.fn(adoptFake),
  release: vi.fn((_id: string): void => {}),
  readOutput: vi.fn(async (_id: string, _lines?: number): Promise<string[]> => []),
  inUse: vi.fn(() => true),

  /** The session spawned last. */
  last(): FakeVorndPty {
    const pty = spawned.at(-1)
    if (!pty) throw new Error('nothing was spawned')
    return pty
  },

  /** Forget every spawn and put back each default. */
  reset(): void {
    spawned.length = 0
    fakeVornd.spawn.mockReset().mockImplementation(spawnFake)
    fakeVornd.adopt.mockReset().mockImplementation(adoptFake)
    fakeVornd.release.mockClear()
    fakeVornd.readOutput.mockReset().mockResolvedValue([])
    fakeVornd.inUse.mockReset().mockReturnValue(true)
  }
}

/** The module `vi.mock` puts in place of `vornd-sessions`. */
export function vorndModule(): Record<string, unknown> {
  return { vorndSessions: fakeVornd, VorndPty: FakeVorndPty }
}
