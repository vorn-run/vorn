import { describe, it, expect, vi, beforeAll, afterAll, beforeEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const log = vi.hoisted(() => ({ info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }))
vi.mock('../packages/server/src/logger', () => ({ default: log }))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => ({ defaults: {} }),
    onChange: () => () => {}
  }
}))

import type { HookEvent, TerminalSession } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { vorndSessions, type SessionNote } from '../packages/server/src/vornd-sessions'
import { hookStatusMapper } from '../packages/server/src/hook-status-mapper'
import { FakeVornd } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

/**
 * Which terminal an agent's hook is linked to, with the Native server switch
 * on: vornd creates the terminals and owns their records, and the link is its
 * to set. Two agents in one folder are told apart by the identity their
 * records carry, as with the switch off. A fake vornd stands in.
 */

const GEN = 'g1'
const CWD = '/shared'

function record(id: string, extra: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id,
    agentType: 'claude',
    projectName: 'p',
    projectPath: CWD,
    status: 'running',
    createdAt: 1,
    pid: 0,
    ...extra
  }
}

function event(name: string, sessionId: string, terminalId?: string): HookEvent {
  return {
    hook_event_name: name,
    session_id: sessionId,
    cwd: CWD,
    ...(terminalId ? { vorn_terminal_id: terminalId } : {})
  }
}

describe('hook linking while vornd owns the terminal records', () => {
  let dataDir: string
  let fake: FakeVornd
  let rev = 1
  const note = (fields: Partial<SessionNote> & { op: SessionNote['op'] }): void => {
    rev += 1
    fake.send('vornd:session', { gen: GEN, rev, ...fields })
  }
  const listed = (id: string): TerminalSession | undefined =>
    ptyManager.getActiveSessions().find((s) => s.id === id)
  const patches = (): Array<Record<string, unknown>> => fake.made('vornd:patch')

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-hook-linking-'))
    fake = new FakeVornd(dataDir)
    fake.native = true
    fake.statuses = true
    fake.terminals = true
    fake.state.registry = {
      gen: GEN,
      rev,
      terminals: [],
      headless: [],
      order: [],
      holds: {},
      nativeHolds: {}
    }
    await fake.start()
    expect(await vorndSessions.connect(fake.endpoint)).toBe(true)
    expect(vorndSessions.createsTerminals()).toBe(true)

    const created: Array<[string, Partial<TerminalSession>]> = [
      ['older', { createdAt: 1, agentSessionId: 'conv-older' }],
      ['newer', { createdAt: 2, agentSessionId: 'conv-newer' }],
      ['plain-1', { agentType: 'gemini', projectPath: '/plain', createdAt: 3 }],
      ['plain-2', { agentType: 'gemini', projectPath: '/plain', createdAt: 4 }]
    ]
    for (const [id, extra] of created) {
      note({
        op: 'upsert',
        kind: 'terminal',
        record: record(id, extra),
        native: true,
        created: true
      })
    }
    await until('the terminals to be taken on', () => listed('plain-2') !== undefined)
  })

  afterAll(async () => {
    vorndSessions.close()
    await fake.stop()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  beforeEach(() => {
    hookStatusMapper.clear()
    log.warn.mockClear()
    fake.calls.length = 0
  })

  it('links the older agent to its own terminal and asks vornd to record it', async () => {
    expect(hookStatusMapper.mapEventToStatus(event('SessionStart', 'conv-older'))).toEqual({
      terminalId: 'older',
      status: 'running'
    })
    expect(hookStatusMapper.mapEventToStatus(event('SessionStart', 'conv-newer'))).toEqual({
      terminalId: 'newer',
      status: 'running'
    })
    await until('the links to reach vornd', () => patches().length === 2)
    expect(patches()).toEqual([
      { id: 'older', fields: { hookSessionId: 'conv-older' } },
      { id: 'newer', fields: { hookSessionId: 'conv-newer' } }
    ])
    expect(log.warn).not.toHaveBeenCalled()
  })

  it('links by the terminal id a hook carries', () => {
    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'cleared', 'older'))).toBe('older')
  })

  it('guesses the most recently launched terminal only when nothing exact exists', () => {
    const guessed = hookStatusMapper.resolveTerminal({
      hook_event_name: 'PreToolUse',
      session_id: 'unknown',
      cwd: '/plain'
    })
    expect(guessed).toBe('plain-2')
    expect(log.warn).toHaveBeenCalledTimes(1)
    expect(String(log.warn.mock.calls[0][0])).toContain('plain-1')
  })
})
