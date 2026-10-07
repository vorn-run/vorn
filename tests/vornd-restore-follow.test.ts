import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => ({ defaults: {} }),
    onChange: () => () => {}
  }
}))
const saved = vi.hoisted(() => ({ calls: 0 }))
vi.mock('../packages/server/src/database', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../packages/server/src/database')>()),
  saveSessions: () => {
    saved.calls += 1
  }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC, type HeadlessSession, type TerminalSession } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { headlessManager } from '../packages/server/src/headless-manager'
import { sessionManager } from '../packages/server/src/session-persistence'
import { seedRestored, listRestored } from '../packages/server/src/restored-sessions'
import { vorndSessions, type SessionNote } from '../packages/server/src/vornd-sessions'
import { wireVorndRestore } from '../packages/server/src/vornd-restore'
import { FakeVornd, effect } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

/**
 * The server following vornd while vornd owns the session records between
 * runs: the terminals and headless agents vornd's copy carried and the holder
 * still holds are taken on from the copy, a session vornd started again under
 * its id replaces the record it had, one let go of for a conversation running
 * elsewhere goes quietly, the list of sessions still offered is mirrored, the
 * database's own records are handed over once, and nothing is saved here. A
 * fake vornd stands in, so each note can be sent on its own.
 */

const GEN = 'g1'

function terminal(id: string, extra: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id,
    agentType: 'shell',
    projectName: 'p',
    projectPath: '/p',
    status: 'running',
    createdAt: 1,
    pid: 0,
    ...extra
  }
}

function headless(id: string, extra: Partial<HeadlessSession> = {}): HeadlessSession {
  return {
    id,
    pid: 0,
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    status: 'running',
    startedAt: 1,
    ...extra
  }
}

describe('the records vornd owns between runs, followed here', () => {
  let dataDir: string
  let fake: FakeVornd
  let rev = 1
  const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
  const announced: TerminalSession[] = []
  const events: string[] = []
  const toldOn = (channel: string): Record<string, unknown>[] =>
    messages.filter((m) => m.channel === channel).map((m) => m.payload)
  const note = (fields: Partial<SessionNote> & { op: SessionNote['op'] }): void => {
    rev += 1
    fake.send('vornd:session', { gen: GEN, rev, ...fields })
  }
  const listed = (id: string): TerminalSession | undefined =>
    ptyManager.getActiveSessions().find((s) => s.id === id)

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-restore-'))
    initDatabase(dataDir)
    fake = new FakeVornd(dataDir)
    fake.native = true
    fake.statuses = true
    fake.terminals = true
    fake.headless = true
    fake.restores = true
    fake.carried = 1
    // What vornd carried and the holder still holds: a terminal with its
    // name and group, and a headless agent; and one session only offered.
    fake.state.registry = {
      gen: GEN,
      rev,
      terminals: [
        {
          ...terminal('held', { pid: 42, displayName: 'Build', groupId: 'g', rev: 1 } as never)
        }
      ],
      headless: [headless('agent', { pid: 43, rev: 1 } as never)],
      order: ['held'],
      holds: {},
      restored: [
        {
          session: terminal('cold', { status: 'idle', savedAt: 5 }),
          endedAt: 5,
          replayable: false,
          partial: false,
          closedCleanly: false,
          rebooted: false
        }
      ]
    }
    fake.state.sessions = [
      { id: 'held', kind: 'pty', pid: 42, status: null, cwd: null, exit: null },
      { id: 'agent', kind: 'piped', pid: 43, status: null, cwd: null, exit: null },
      { id: 'stranger', kind: 'pty', pid: 44, status: null, cwd: null, exit: null }
    ]
    await fake.start()
    ptyManager.on('client-message', (channel: string, payload: Record<string, unknown>) =>
      messages.push({ channel, payload })
    )
    for (const name of ['session-created', 'session-exit']) {
      ptyManager.on(name, () => events.push(name))
    }
    wireVorndRestore((session) => announced.push(session))
    sessionManager.setOwnedElsewhere(() => vorndSessions.restoresSessions())
    // What this server's database kept of the last run, handed over once.
    seedRestored([terminal('from-db', { savedAt: 7 })], 10)
    expect(await vorndSessions.connect(fake.endpoint)).toBe(true)
    expect(vorndSessions.restoresSessions()).toBe(true)
  })

  afterAll(async () => {
    vorndSessions.close()
    await fake.stop()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('takes on what the holder holds from the copy, and hands the database its records once', async () => {
    await until('the terminal to be taken on', () => listed('held') !== undefined)
    expect(listed('held')).toMatchObject({ pid: 42, displayName: 'Build', groupId: 'g' })
    expect(listed('held')).not.toHaveProperty('rev')
    expect(ptyManager.hasLivePty('held')).toBe(true)
    expect(announced.map((s) => s.id)).toEqual(['held'])
    // Not a terminal vornd created: nothing of a new session runs for it.
    expect(events).toEqual([])
    // The headless agent is followed again, its output read from the start.
    await until('the agent to be followed', () =>
      headlessManager.getActiveSessions().some((s) => s.id === 'agent')
    )
    await until('its output to be read', () =>
      fake.made('terminal:attach').some((p) => p.id === 'agent')
    )
    expect(headlessManager.getActiveSessions().find((s) => s.id === 'agent')?.pid).toBe(43)
    // One no record names is left where it is.
    expect(listed('stranger')).toBeUndefined()
    // The subscription said when the machine came up.
    expect(fake.made('vornd:subscribe')[0]).toEqual({ bootTime: expect.any(Number) })
    // The database's records went to vornd, and are offered from here no more.
    await until('the records to be handed over', () => fake.made('vornd:carry').length === 1)
    expect((fake.made('vornd:carry')[0].terminals as TerminalSession[]).map((s) => s.id)).toEqual([
      'from-db'
    ])
    expect(listRestored()).toEqual([])
    // vornd's offers are mirrored.
    expect(vorndSessions.mirror.restored().map((r) => r.session.id)).toEqual(['cold'])
  })

  it('saves nothing to its own session records while vornd owns them', async () => {
    const before = saved.calls
    sessionManager.startAutoSave(() => ptyManager.getActiveSessions())
    sessionManager.persistNow()
    sessionManager.clear()
    await new Promise((r) => setTimeout(r, 20))
    expect(saved.calls).toBe(before)
  })

  it('follows the offers as vornd changes them', async () => {
    note({ op: 'restored', restored: [] })
    await until('the offer to go', () => vorndSessions.mirror.restored().length === 0)
    expect(vorndSessions.mirror.revision?.rev).toBe(rev)
  })

  it('replaces a record vornd started again under its id, as a terminal it created', async () => {
    // The held shell's program ended: its record stays, idle.
    fake.send('vornd:effect', effect('held', 'exit', 3, { exitCode: 0 }))
    await until('the exit', () => !ptyManager.hasLivePty('held'))
    expect(listed('held')?.status).toBe('idle')
    const order = ptyManager.getActiveSessions().map((s) => s.id)

    note({
      op: 'upsert',
      kind: 'terminal',
      record: terminal('held', { displayName: 'Build', groupId: 'g' }),
      native: true,
      created: true,
      resumed: true
    })
    await until('the resumed record', () => ptyManager.hasLivePty('held'))
    expect(listed('held')).toMatchObject({ status: 'running', displayName: 'Build', groupId: 'g' })
    expect(events).toEqual(['session-exit', 'session-created'])
    expect(toldOn(IPC.TERMINAL_EXIT).map((p) => p.id)).toEqual(['held'])
    expect(order).toEqual(['held'])
    note({
      op: 'upsert',
      kind: 'terminal',
      record: terminal('held', { pid: 50 }),
      native: true,
      started: { pid: 50, epoch: 8 }
    })
    await until('its program', () => listed('held')?.pid === 50)
  })

  it('lets go of a record released for a conversation running elsewhere without an exit', async () => {
    const exits = toldOn(IPC.TERMINAL_EXIT).length
    note({ op: 'remove', kind: 'terminal', id: 'held', native: true, released: true })
    await until('the record to go', () => listed('held') === undefined)
    expect(vorndSessions.get('held')).toBeUndefined()
    expect(toldOn(IPC.TERMINAL_EXIT)).toHaveLength(exits)
    expect(events.filter((e) => e === 'session-exit')).toHaveLength(1)
  })

  it('takes a terminal vornd adopted after a reconnect from the subscription, not the note', async () => {
    note({
      op: 'upsert',
      kind: 'terminal',
      record: terminal('later', { pid: 60 }),
      native: true,
      created: true,
      adopted: true,
      started: { pid: 60, epoch: 9 }
    })
    await new Promise((r) => setTimeout(r, 50))
    expect(listed('later')).toBeUndefined()
    fake.state.registry = {
      ...(fake.state.registry as object),
      rev,
      terminals: [terminal('later', { pid: 60 })],
      headless: [],
      order: ['later']
    } as never
    fake.state.sessions = [
      { id: 'later', kind: 'pty', pid: 60, status: null, cwd: null, exit: null }
    ]
    fake.send('vornd:connected', {})
    await until('the terminal to be taken on', () => listed('later')?.pid === 60)
    expect(ptyManager.hasLivePty('later')).toBe(true)
  })
})
