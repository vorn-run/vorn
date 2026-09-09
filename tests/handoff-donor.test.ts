import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { ChildProcess } from 'node:child_process'
import { HANDOFF_PROTOCOL_VERSION, type HandoffRequest } from '@vornrun/shared/protocol'
import {
  handOver,
  resetHandoffForTests,
  type HandoffHost,
  type DonorPane
} from '../packages/server/src/handoff/donor'
import { readManifest, type HandoffManifest } from '../packages/server/src/handoff/manifest'

/**
 * The transaction, against a replacement that fails on cue. What needs proving is
 * the failures: before the commit, indistinguishable from never having tried;
 * after it, recovered from; and nowhere a pty killed.
 */

/** A replacement that says what it is told to say, when it is told to say it. */
class FakeHeir extends EventEmitter {
  pid = 4242
  killed: string | null = null
  sent: unknown[] = []
  send(msg: unknown): boolean {
    this.sent.push(msg)
    return true
  }
  kill(signal: string): boolean {
    this.killed = signal
    return true
  }
  unref(): void {}
  disconnect(): void {}
}

function pane(id: string, fd: number): DonorPane {
  return {
    // Only the fields the manifest carries are read here.
    session: { id, cols: 80, rows: 24 } as DonorPane['session'],
    fd,
    pid: 1000 + fd,
    cols: 80,
    rows: 24
  }
}

interface Recorded {
  host: HandoffHost
  events: string[]
  dataDir: string
}

function makeHost(over: Partial<HandoffHost> = {}): Recorded {
  const events: string[] = []
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-handoff-'))
  const host: HandoffHost = {
    dataDir,
    describePanes: () => {
      events.push('describe')
      return [pane('a', 20), pane('b', 21)]
    },
    pauseAll: () => events.push('pause'),
    resumeAll: () => events.push('resume'),
    quiesce: async () => {
      events.push('quiesce')
    },
    openLogFd: () => fs.openSync(path.join(dataDir, 'log'), 'a'),
    release: async () => {
      events.push('release')
    },
    reclaim: async () => {
      events.push('reclaim')
      return true
    },
    exit: () => events.push('exit'),
    ...over
  }
  return { host, events, dataDir }
}

const request: HandoffRequest = {
  handoffVersion: HANDOFF_PROTOCOL_VERSION,
  // Any real path passes the "is there something to run" guard; the fake heir is
  // what actually stands in for running it.
  exec: process.execPath,
  args: ['server.js'],
  env: {},
  cwd: os.tmpdir(),
  appVersion: '9.9.9'
}

/** Answer the handshake the donor is waiting on, on the next turn of the loop. */
function replying(...words: string[]): { spawn: () => ChildProcess; heir: FakeHeir } {
  const heir = new FakeHeir()
  return {
    heir,
    spawn: () => {
      for (const word of words) setTimeout(() => heir.emit('message', { kind: word }), 5)
      // The commit reply is only sent once the donor has asked for it.
      return heir as unknown as ChildProcess
    }
  }
}

beforeEach(() => resetHandoffForTests())
afterEach(() => {
  vi.useRealTimers()
  resetHandoffForTests()
})

describe('handing terminals to a replacement', () => {
  it('refuses a caller speaking a different handoff contract', async () => {
    const { host, events } = makeHost()
    const result = await handOver({ ...request, handoffVersion: 99 }, host, () => {
      throw new Error('must not spawn')
    })
    expect(result.kind).toBe('declined')
    // Nothing was touched at all: not even the pause, which is the first thing
    // that would be visible to somebody typing.
    expect(events).toEqual([])
  })

  it('refuses rather than handing over a machine it cannot fully describe', async () => {
    // Invariant 6. Half the terminals arriving is, to the person whose terminals
    // they are, indistinguishable from losing the other half.
    let looked = false
    const { host, events } = makeHost({
      describePanes: () => {
        looked = true
        return null
      }
    })
    const result = await handOver(request, host, () => {
      throw new Error('must not spawn')
    })
    expect(result.kind).toBe('declined')
    // Paused to look, then started again. That round trip is the whole story.
    expect(looked).toBe(true)
    expect(events).toEqual(['pause', 'resume'])
  })

  it('leaves everything untouched when the replacement never takes the panes', async () => {
    const { heir, spawn } = replying()
    const { host, events, dataDir } = makeHost()
    vi.useFakeTimers()
    const pending = handOver(request, host, spawn)
    await vi.advanceTimersByTimeAsync(25_000)
    const result = await pending

    expect(result).toEqual({
      kind: 'declined',
      because: expect.stringContaining('never took the panes')
    })
    // The commit never happened, so the endpoint was never given up and the
    // readers are running again. This is the rollback that has to be complete.
    expect(events).not.toContain('release')
    expect(events).not.toContain('exit')
    expect(events.at(-1)).toBe('resume')
    expect(heir.killed).toBe('SIGKILL')
    // And the manifest is gone, rather than left to be adopted by something later.
    expect(fs.readdirSync(dataDir).filter((f) => f.startsWith('handoff-'))).toEqual([])
  })

  it('commits in order and leaves without killing anything', async () => {
    const { heir, spawn } = replying('imported', 'serving')
    const { host, events, dataDir } = makeHost()

    // Read at the moment the replacement is started, which is the only moment it
    // is guaranteed to be on disk: the heir's reply is what removes it.
    // A holder rather than a bare `let`: the only assignment is inside a callback
    // this function hands off rather than calls, and the checker keeps the
    // initializer's narrowed type across that boundary.
    const captured: { manifest: HandoffManifest | null } = { manifest: null }
    const spawnAndRead = (...args: Parameters<typeof spawn>): ReturnType<typeof spawn> => {
      const found = fs.readdirSync(dataDir).find((f) => f.startsWith('handoff-'))
      if (found) captured.manifest = readManifest(path.join(dataDir, found))
      return spawn(...args)
    }

    const result = await handOver(request, host, spawnAndRead)

    expect(result).toMatchObject({ kind: 'handed-over', sessions: 2, pid: 4242 })
    // The order is the transaction. Everything durable is written down before the
    // replacement starts; the endpoint is released only once it has the panes.
    expect(events.slice(0, 4)).toEqual(['pause', 'describe', 'quiesce', 'release'])
    expect(heir.sent).toEqual([{ kind: 'commit' }])
    // Never resumed: this server is not going to read these panes again, and
    // resuming would put two readers on one pty.
    expect(events).not.toContain('resume')

    // The manifest names both panes, at the slots the stdio array put them in.
    expect(captured.manifest?.panes.map((p) => [p.session.id, p.slot])).toEqual([
      ['a', 4],
      ['b', 5]
    ])

    // The exit is deferred so the reply reaches the caller first.
    await vi.waitFor(() => expect(events).toContain('exit'))
  })

  it('takes the endpoint back when the replacement dies after the commit', async () => {
    // The one genuinely dangerous case. Both processes still hold every pty --
    // nothing has been killed -- so there is something to go back to.
    const { heir, spawn } = replying('imported')
    const { host, events } = makeHost()
    vi.useFakeTimers()
    const pending = handOver(request, host, spawn)
    await vi.advanceTimersByTimeAsync(1_000)
    heir.emit('exit', 1, null)
    const result = await pending

    expect(result).toEqual({
      kind: 'declined',
      because: expect.stringContaining('took its terminals back')
    })
    expect(events).toContain('release')
    expect(events).toContain('reclaim')
    // Reading again, under the old server, which is the point of reclaiming.
    expect(events.at(-1)).toBe('resume')
    expect(events).not.toContain('exit')
  })

  it('says so plainly when it cannot take the endpoint back', async () => {
    const { heir, spawn } = replying('imported')
    const { host } = makeHost({
      reclaim: async () => false
    })
    vi.useFakeTimers()
    const pending = handOver(request, host, spawn)
    await vi.advanceTimersByTimeAsync(1_000)
    heir.emit('exit', 1, null)
    const result = await pending

    // The terminals are alive and unreachable, and a person has to be told both
    // halves of that rather than only the reassuring one.
    expect(result).toEqual({
      kind: 'declined',
      because: expect.stringContaining('Vorn must be reopened')
    })
  })

  it('refuses a second handoff while one is running', async () => {
    const { spawn } = replying()
    const { host } = makeHost()
    vi.useFakeTimers()
    const first = handOver(request, host, spawn)
    const second = await handOver(request, host, () => {
      throw new Error('must not spawn')
    })
    expect(second).toEqual({ kind: 'declined', because: 'a handoff is already running' })
    await vi.advanceTimersByTimeAsync(25_000)
    await first
  })
})
