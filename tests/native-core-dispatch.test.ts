import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'

/**
 * `VORN_CORE=native`: the screen model and the output analysis go to the core.
 *
 * The core itself is replaced by fakes that record what they are handed, so this
 * checks the dispatch -- which calls reach the core, what comes back, and that a
 * core that throws costs the session its model rather than the server -- on a
 * machine with no Rust toolchain. The core's own behaviour is tested in Rust.
 */

const { fakeCore, screens, analyzers } = vi.hoisted(() => {
  const screens: FakeScreen[] = []
  const analyzers: FakeAnalyzer[] = []

  class FakeScreen {
    fed: string[] = []
    size: [number, number]
    cwd = ''
    title = ''
    failNext = false
    freed = false
    constructor(cols: number, rows: number) {
      this.size = [cols, rows]
      screens.push(this)
    }
    /** The core's contract: returns the cwd an OSC 5522 moved to, else null. */
    feed(data: string): string | null {
      if (this.failNext) throw new Error('core fault')
      this.fed.push(data)
      // eslint-disable-next-line no-control-regex
      const m = /\x1b\]5522;cwd;([^\x07]*)\x07/.exec(data)
      if (m && m[1] !== this.cwd) {
        this.cwd = m[1]
        return m[1]
      }
      return null
    }
    restoreLabels(title?: string | null, cwd?: string | null): void {
      if (title) this.title = title
      if (cwd) this.cwd = cwd
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
    Screen: FakeScreen,
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
  activeCore: () => ({ mode: 'native', native: fakeCore }),
  coreFor: () => fakeCore
}))

import {
  createScreen,
  feedScreen,
  hasScreen,
  resetScreens,
  resizeScreen,
  serializeScreen,
  setCwdReporter
} from '../packages/server/src/terminal-screen'
import { ptyManager } from '../packages/server/src/pty-manager'

interface Internals {
  sessions: Map<string, TerminalSession>
  appendOutput(id: string, data: string): void
  clearSessionTracking(id: string): void
}
const pm = ptyManager as unknown as Internals

beforeEach(() => {
  resetScreens()
  screens.length = 0
  analyzers.length = 0
  setCwdReporter(null)
})

describe('the screen model on the native core', () => {
  it('feeds, resizes and serializes through the core', async () => {
    createScreen('s', 80, 24)
    feedScreen('s', 'hello')
    await resizeScreen('s', 100, 30)
    const snap = await serializeScreen('s')
    expect(screens).toHaveLength(1)
    expect(snap).toEqual({ screen: 'hello', cols: 100, rows: 30, title: '', cwd: '' })
  })

  it('reports a new cwd when an OSC moves it, and only then', () => {
    const reported: string[] = []
    setCwdReporter((_, cwd) => reported.push(cwd))
    createScreen('s', 80, 24)
    feedScreen('s', '\x1b]5522;cwd;/tmp/a\x07')
    feedScreen('s', '\x1b]5522;cwd;/tmp/a\x07')
    feedScreen('s', 'plain')
    feedScreen('s', '\x1b]5522;cwd;/tmp/b\x07')
    expect(reported).toEqual(['/tmp/a', '/tmp/b'])
  })

  it('puts restored labels back on a native screen', async () => {
    createScreen('r', 80, 24, { title: 'vim', cwd: '/srv' })
    const snap = await serializeScreen('r')
    expect(snap?.title).toBe('vim')
    expect(snap?.cwd).toBe('/srv')
  })

  it('frees the native terminal when the screen is cleared', () => {
    createScreen('f', 80, 24)
    resetScreens()
    expect(screens[0].freed).toBe(true)
  })

  it('drops the model, not the session, when the core throws', async () => {
    createScreen('feed', 80, 24)
    screens[0].failNext = true
    feedScreen('feed', 'x')
    expect(hasScreen('feed')).toBe(false)

    createScreen('resize', 80, 24)
    screens[1].failNext = true
    await resizeScreen('resize', 90, 24)
    expect(hasScreen('resize')).toBe(false)

    createScreen('serialize', 80, 24)
    screens[2].failNext = true
    expect(await serializeScreen('serialize')).toBeNull()
    expect(hasScreen('serialize')).toBe(false)
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

  it('hands each raw chunk to one analyzer per session and takes its status', () => {
    const session = addSession('a')
    pm.appendOutput('a', 'one\n')
    analyzers[0].next = 2 // waiting
    pm.appendOutput('a', 'two? ')
    expect(analyzers).toHaveLength(1)
    expect(analyzers[0].calls).toEqual([
      ['one\n', true],
      ['two? ', true]
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

  it('reads all lines for getOutput(id, 0), as the JS path does', () => {
    addSession('z')
    pm.appendOutput('z', 'one\n')
    pm.appendOutput('z', 'two\n')
    expect(ptyManager.getOutput('z', 0)).toEqual(['one', 'two'])
    pm.clearSessionTracking('z')
    pm.sessions.delete('z')
  })

  it('falls back to the JS analysis when the core throws, instead of throwing from the pty', () => {
    addSession('t')
    addSession('u')
    pm.appendOutput('t', 'first\n')
    pm.appendOutput('u', 'other\npar')
    analyzers[0].failNext = true
    expect(() => pm.appendOutput('t', 'second\n')).not.toThrow()
    // Output read before the fault survives, for every session, and the chunk
    // the core already took is not added twice.
    expect(ptyManager.getOutput('t')).toEqual(['first', 'second'])
    pm.appendOutput('t', 'third\n')
    expect(ptyManager.getOutput('t')).toEqual(['first', 'second', 'third'])
    pm.appendOutput('u', 'tial\n')
    expect(ptyManager.getOutput('u')).toEqual(['other', 'partial'])
    expect(analyzers.every((a) => a.freed)).toBe(true)
    for (const id of ['t', 'u']) {
      pm.clearSessionTracking(id)
      pm.sessions.delete(id)
    }
  })
})
