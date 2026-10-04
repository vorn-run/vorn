import { MAX_FLUSH_UNITS } from '../packages/server/src/output-buffer'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'

/**
 * The screen model and the output analysis go to the core.
 *
 * The core itself is replaced by fakes that record what they are handed, so this
 * checks the dispatch -- which calls reach the core, what comes back, and that a
 * core that throws costs the session its model or its status rather than the
 * server -- on a machine with no Rust toolchain. The core's own behaviour is
 * tested in Rust, and against the real binary in `core-pipeline.test.ts`.
 */

const switches = vi.hoisted(() => ({ core: true, failSeed: false }))

const { fakeCore, screens, analyzers } = vi.hoisted(() => {
  const screens: FakePipeline[] = []
  const analyzers: FakeAnalyzer[] = []

  /** A pipeline that does its work at once, so a read needs no waiting. */
  class FakePipeline {
    fed: string[] = []
    size: [number, number]
    cwd = ''
    title = ''
    failNext = false
    freed = false
    scrollbackSeed = ''
    constructor(
      cols: number,
      rows: number,
      readonly onEvent: (event: { kind: string; cwd?: string | null }) => void
    ) {
      this.size = [cols, rows]
      screens.push(this)
    }
    /** A bare `!` stands in for a BEL here. */
    feedScreen(data: string): void {
      if (this.failNext) throw new Error('core fault')
      this.fed.push(data)
      // eslint-disable-next-line no-control-regex
      const m = /\x1b\]5522;cwd;([^\x07]*)\x07/.exec(data)
      if (m && m[1] !== this.cwd) {
        this.cwd = m[1]
        this.onEvent({ kind: 'cwd', cwd: m[1] })
      }
      if (data.includes('!')) this.onEvent({ kind: 'bell' })
    }
    restoreLabels(title?: string | null, cwd?: string | null): void {
      if (title) this.title = title
      if (cwd) this.cwd = cwd
    }
    seedScrollback(data: string): void {
      if (switches.failSeed) throw new Error('core fault')
      this.scrollbackSeed = data
    }
    scrollback(): string {
      return this.scrollbackSeed + this.fed.join('')
    }
    free(): void {
      this.freed = true
    }
    resize(cols: number, rows: number): void {
      if (this.failNext) throw new Error('core fault')
      this.size = [cols, rows]
    }
    serialize(): { screen: string; cols: number; rows: number; title: string; cwd: string } {
      if (this.failNext) throw new Error('core fault')
      return {
        screen: this.fed.join(''),
        cols: this.size[0],
        rows: this.size[1],
        title: this.title,
        cwd: this.cwd
      }
    }
  }

  class FakeAnalyzer {
    calls: Array<[string, boolean]> = []
    next = 0
    failNext = false
    freed = false
    constructor() {
      analyzers.push(this)
    }
    append(data: string, analyze: boolean): number {
      // The worst case: the chunk is taken in, then the status step throws.
      this.calls.push([data, analyze])
      if (this.failNext) throw new Error('core fault')
      return this.next
    }
    free(): void {
      this.freed = true
    }
    /** Completed lines without their `\n`, as the real analyzer keeps them. */
    output(lines?: number): string[] {
      const all = this.calls
        .map(([d]) => d)
        .join('')
        .split('\n')
      all.pop()
      return lines ? all.slice(-lines) : all
    }
    partial(): string {
      return this.calls
        .map(([d]) => d)
        .join('')
        .split('\n')
        .pop()!
    }
  }

  const fakeCore = {
    info: () => ({ version: 'test', ghostty: null }),
    hello: () => 'hello',
    TerminalPipeline: FakePipeline,
    Analyzer: FakeAnalyzer
  }
  return { fakeCore, screens, analyzers }
})

vi.mock('node-pty', () => ({ spawn: vi.fn() }))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/native-core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../packages/server/src/native-core')>()),
  activeCore: () => ({ native: switches.core ? fakeCore : null }),
  nativeCore: () => (switches.core ? fakeCore : null)
}))

import {
  createScreen,
  feedScreen,
  hasScreen,
  resetScreens,
  resizeScreen,
  serializeScreen,
  setBellReporter,
  setCwdReporter
} from '../packages/server/src/terminal-screen'
import { ptyManager } from '../packages/server/src/pty-manager'
import {
  appendScrollback,
  readScrollback,
  resetScrollback
} from '../packages/server/src/terminal-scrollback'

interface Internals {
  sessions: Map<string, TerminalSession>
  appendOutput(id: string, data: string): void
  flushAnalysis(id: string): void
  clearSessionTracking(id: string): void
}
const pm = ptyManager as unknown as Internals

beforeEach(() => {
  resetScreens()
  screens.length = 0
  analyzers.length = 0
  switches.core = true
  switches.failSeed = false
  resetScrollback()
  setCwdReporter(null)
  setBellReporter(null)
})

describe('the screen model on the native core', () => {
  it('feeds, resizes and serializes through the core', async () => {
    createScreen('s', 80, 24)
    feedScreen('s', 'hello')
    resizeScreen('s', 100, 30)
    const snap = await serializeScreen('s')
    expect(screens).toHaveLength(1)
    expect(snap).toEqual({ screen: 'hello', cols: 100, rows: 30, title: '', cwd: '' })
  })

  it('passes on what the terminal reports: a new cwd, and the bell', () => {
    const reported: string[] = []
    setCwdReporter((_, cwd) => reported.push(cwd))
    setBellReporter((id) => reported.push(`bell ${id}`))
    createScreen('s', 80, 24)
    feedScreen('s', '\x1b]5522;cwd;/tmp/a\x07')
    feedScreen('s', 'plain')
    feedScreen('s', 'ding!')
    expect(reported).toEqual(['/tmp/a', 'bell s'])
  })

  it('answers a feed with false when the model took it, and null when there is none', () => {
    createScreen('b', 80, 24)
    expect(feedScreen('b', 'quiet')).toBe(false)
    expect(feedScreen('none', 'x')).toBeNull()
  })

  it('keeps no model when the core is not loaded', async () => {
    switches.core = false
    createScreen('n', 80, 24)
    expect(hasScreen('n')).toBe(false)
    expect(feedScreen('n', 'x')).toBeNull()
    expect(await serializeScreen('n')).toBeNull()
  })

  it('puts restored labels back on the screen', async () => {
    createScreen('r', 80, 24, { title: 'vim', cwd: '/srv' })
    const snap = await serializeScreen('r')
    expect(snap?.title).toBe('vim')
    expect(snap?.cwd).toBe('/srv')
  })

  it('stops the terminal thread when the screen is cleared', () => {
    createScreen('f', 80, 24)
    resetScreens()
    expect(screens[0].freed).toBe(true)
  })

  it('stops a pipeline that fails as it starts, and keeps the scrollback', () => {
    appendScrollback('seed', 'before')
    switches.failSeed = true
    createScreen('seed', 80, 24)
    expect(hasScreen('seed')).toBe(false)
    expect(screens[0].freed).toBe(true)
    expect(readScrollback('seed')).toBe('before')
  })

  it('drops the model, not the session, when the core throws', async () => {
    createScreen('feed', 80, 24)
    screens[0].failNext = true
    feedScreen('feed', 'x')
    expect(hasScreen('feed')).toBe(false)

    createScreen('resize', 80, 24)
    screens[1].failNext = true
    resizeScreen('resize', 90, 24)
    expect(hasScreen('resize')).toBe(false)

    // A serialize that fails keeps the pipeline: its scrollback and history
    // carry on without a screen.
    createScreen('serialize', 80, 24)
    screens[2].failNext = true
    expect(await serializeScreen('serialize')).toBeNull()
    expect(hasScreen('serialize')).toBe(true)
  })
})

describe('output analysis on the native core', () => {
  function addSession(id: string, statusSource?: 'hooks'): TerminalSession {
    const session = {
      id,
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp/p',
      status: 'running',
      statusSource,
      createdAt: Date.now(),
      pid: 0
    } as unknown as TerminalSession
    pm.sessions.set(id, session)
    return session
  }

  it('analyzes the first read at once and the rest of a burst together', () => {
    const session = addSession('a')
    pm.appendOutput('a', 'one\n')
    // Quiet before it, so it went straight in.
    expect(analyzers[0].calls).toEqual([['one\n', true]])
    pm.appendOutput('a', 'and ')
    analyzers[0].next = 2 // waiting
    pm.appendOutput('a', 'two? ')
    // The rest of the burst waits for the window.
    expect(analyzers[0].calls).toHaveLength(1)
    pm.flushAnalysis('a')
    expect(analyzers).toHaveLength(1)
    expect(analyzers[0].calls).toEqual([
      ['one\n', true],
      ['and two? ', true]
    ])
    expect(session.status).toBe('waiting')
    expect(ptyManager.getOutput('a', 1)).toEqual(['one'])
    pm.clearSessionTracking('a')
    pm.sessions.delete('a')
  })

  it('asks for bracketed paste only on a hook-driven session', () => {
    const session = addSession('h', 'hooks')
    pm.appendOutput('h', 'x')
    expect(analyzers[0].calls).toEqual([['x', false]])
    expect(session.status).toBe('running')
    pm.clearSessionTracking('h')
    expect(analyzers[0].freed).toBe(true)
    pm.sessions.delete('h')
  })

  it('analyzes the rest of a burst on its own, and before getOutput reads', async () => {
    const session = addSession('w')
    pm.appendOutput('w', 'ready\n')
    pm.appendOutput('w', 'set\n')
    expect(ptyManager.getOutput('w')).toEqual(['ready', 'set'])
    analyzers[0].next = 2
    pm.appendOutput('w', 'more? ')
    await new Promise((r) => setTimeout(r, 30))
    expect(session.status).toBe('waiting')
    pm.clearSessionTracking('w')
    pm.sessions.delete('w')
  })

  it('analyzes held output before a hook sets the status, so the hook has the last word', () => {
    const session = addSession('k')
    pm.appendOutput('k', 'one\n')
    analyzers[0].next = 2 // waiting
    pm.appendOutput('k', 'Allow? ')
    expect(analyzers[0].calls).toHaveLength(1)
    ptyManager.updateSessionStatus('k', 'running')
    ptyManager.promoteToHookStatus('k')
    expect(analyzers[0].calls).toEqual([
      ['one\n', true],
      ['Allow? ', true]
    ])
    pm.flushAnalysis('k')
    expect(session.status).toBe('running')
    pm.clearSessionTracking('k')
    pm.sessions.delete('k')
  })

  it('analyzes held output before input decides the session is active', () => {
    const session = addSession('i')
    pm.appendOutput('i', 'one\n')
    analyzers[0].next = 2 // waiting
    pm.appendOutput('i', 'Continue? ')
    ptyManager.writeToPty('i', 'y')
    pm.flushAnalysis('i')
    // The prompt was seen first, so the answer moves the session on.
    expect(session.status).toBe('running')
    pm.clearSessionTracking('i')
    pm.sessions.delete('i')
  })

  it('takes status from the reads after a bracketed-paste switch, as per-read analysis does', () => {
    const session = addSession('b')
    pm.appendOutput('b', 'start\n')
    pm.appendOutput('b', 'out\x1b[?2004h> ')
    pm.appendOutput('b', 'thinking')
    pm.appendOutput('b', ' more')
    analyzers[0].next = 1 // running, from the patterns on the reads after the switch
    pm.flushAnalysis('b')
    // The switch ends one batch and the reads after it make the next, so the
    // core cannot let the switch decide for the reads that followed it.
    expect(analyzers[0].calls).toHaveLength(2)
    pm.flushAnalysis('b')
    expect(analyzers[0].calls).toEqual([
      ['start\n', true],
      ['out\x1b[?2004h> ', true],
      ['thinking more', true]
    ])
    expect(session.status).toBe('running')
    pm.clearSessionTracking('b')
    pm.sessions.delete('b')
  })

  it('analyzes a batch that ends with the switch in one call', () => {
    addSession('c')
    pm.appendOutput('c', 'start\n')
    pm.appendOutput('c', 'out ')
    pm.appendOutput('c', '\x1b[?2004h> ')
    pm.flushAnalysis('c')
    expect(analyzers[0].calls).toEqual([
      ['start\n', true],
      ['out \x1b[?2004h> ', true]
    ])
    pm.clearSessionTracking('c')
    pm.sessions.delete('c')
  })

  it('analyzes a burst in batches of at most 64 KB, one per turn, and all of it for getOutput', async () => {
    addSession('big')
    pm.appendOutput('big', 'a\n')
    const burst = 'x'.repeat(3 * MAX_FLUSH_UNITS) + '\n'
    pm.appendOutput('big', burst)
    const analyzer = analyzers.at(-1)!
    expect(analyzer.calls).toHaveLength(1)

    await new Promise((r) => setImmediate(r))
    expect(analyzer.calls).toHaveLength(2)
    expect(analyzer.calls[1]![0]).toHaveLength(MAX_FLUSH_UNITS)

    expect(ptyManager.getOutput('big')).toEqual(['a', 'x'.repeat(3 * MAX_FLUSH_UNITS)])
    expect(analyzer.calls.every(([d]) => d.length <= MAX_FLUSH_UNITS)).toBe(true)
    pm.clearSessionTracking('big')
    pm.sessions.delete('big')
  })

  it('analyzes nothing, but still goes idle, when the core is not loaded', () => {
    vi.useFakeTimers()
    try {
      switches.core = false
      const session = addSession('none')
      pm.appendOutput('none', 'Continue? (y/n)')
      expect(analyzers).toHaveLength(0)
      expect(ptyManager.getOutput('none')).toEqual([])
      expect(session.status).toBe('running')
      vi.advanceTimersByTime(5000)
      expect(session.status).toBe('idle')
      pm.clearSessionTracking('none')
      pm.sessions.delete('none')
    } finally {
      vi.useRealTimers()
    }
  })

  it('reads all lines for getOutput(id, 0)', () => {
    addSession('z')
    pm.appendOutput('z', 'one\n')
    pm.appendOutput('z', 'two\n')
    expect(ptyManager.getOutput('z', 0)).toEqual(['one', 'two'])
    pm.clearSessionTracking('z')
    pm.sessions.delete('z')
  })

  it('stops analyzing a session whose analyzer throws, instead of throwing from the pty', () => {
    addSession('t')
    addSession('u')
    pm.appendOutput('t', 'first\n')
    pm.appendOutput('u', 'other\n')
    const [t, u] = analyzers
    t.failNext = true
    pm.appendOutput('t', 'second\n')
    expect(() => pm.flushAnalysis('t')).not.toThrow()
    expect(t.freed).toBe(true)
    // The session goes on without status or output lines; nothing is asked of
    // the faulted analyzer again, and no new one is made for it.
    pm.appendOutput('t', 'third\n')
    expect(ptyManager.getOutput('t')).toEqual([])
    expect(analyzers).toHaveLength(2)
    // The other session is untouched.
    pm.appendOutput('u', 'more\n')
    expect(ptyManager.getOutput('u')).toEqual(['other', 'more'])
    expect(u.freed).toBe(false)
    for (const id of ['t', 'u']) {
      pm.clearSessionTracking(id)
      pm.sessions.delete(id)
    }
  })
})
