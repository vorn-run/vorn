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

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC, type TerminalSession } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { vorndSessions, type SessionNote } from '../packages/server/src/vornd-sessions'
import { isWorkspaceHeld } from '../packages/server/src/workspace-holds'
import { FakeVornd, effect } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

/**
 * The server following the terminals vornd creates and changes itself, from
 * the registry notes a vornd that creates terminals tells: a terminal taken
 * on and listed, its program's start and end, a rename and an order told to
 * clients, the directories vornd holds, a close, and the claims and the
 * winding-down vornd is told of. A fake vornd stands in, so each note can be
 * sent on its own.
 */

const GEN = 'g1'

function record(id: string, extra: Partial<TerminalSession> = {}): TerminalSession {
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

describe('the terminals vornd creates and changes, followed here', () => {
  let dataDir: string
  let fake: FakeVornd
  let rev = 1
  const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
  const events: Array<{ name: string; args: unknown[] }> = []
  const toldOn = (channel: string): Record<string, unknown>[] =>
    messages.filter((m) => m.channel === channel).map((m) => m.payload)
  const note = (fields: Partial<SessionNote> & { op: SessionNote['op'] }): void => {
    rev += 1
    fake.send('vornd:session', { gen: GEN, rev, ...fields })
  }
  const listed = (id: string): TerminalSession | undefined =>
    ptyManager.getActiveSessions().find((s) => s.id === id)

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-follow-'))
    initDatabase(dataDir)
    fake = new FakeVornd(dataDir)
    fake.state.registry = {
      gen: GEN,
      rev,
      terminals: [],
      headless: [],
      order: [],
      holds: {},
      nativeHolds: { '/held-at-start': 1 }
    }
    await fake.start()
    ptyManager.on('client-message', (channel: string, payload: Record<string, unknown>) =>
      messages.push({ channel, payload })
    )
    for (const name of ['session-created', 'session-exit', 'session-renamed', 'records-changed']) {
      ptyManager.on(name, (...args: unknown[]) => events.push({ name, args }))
    }
    vorndSessions.setClosingSource(() => closing)
    expect(await vorndSessions.connect(fake.endpoint)).toBe(true)
  })

  afterAll(async () => {
    vorndSessions.close()
    await fake.stop()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  let closing = { draining: false, handingOver: false }

  it('takes on a terminal vornd created, follows its program and tells its changes', async () => {
    expect(isWorkspaceHeld('/held-at-start')).toBe(true)

    // Created: listed last, and what hangs off a new session runs.
    note({ op: 'upsert', kind: 'terminal', record: record('n'), native: true, created: true })
    await until('the terminal to be taken on', () => listed('n') !== undefined)
    expect(ptyManager.getActiveSessions().map((s) => s.id)).toEqual(['n'])
    expect(events.filter((e) => e.name === 'session-created')).toHaveLength(1)
    expect(ptyManager.hasLivePty('n')).toBe(true)
    // Told once: a snapshot told again does not create it twice.
    note({ op: 'upsert', kind: 'terminal', record: record('n'), native: true, created: true })

    // Its program is up.
    note({
      op: 'upsert',
      kind: 'terminal',
      record: record('n', { pid: 42 }),
      native: true,
      started: { pid: 42, epoch: 7 }
    })
    await until('the pid', () => listed('n')?.pid === 42)

    // A rename and a group vornd set for a client: told as the server's own are.
    note({
      op: 'upsert',
      kind: 'terminal',
      record: record('n', { pid: 42, displayName: 'mine', renamedByPerson: true, groupId: 'g' }),
      native: true
    })
    await until('the rename', () => listed('n')?.displayName === 'mine')
    expect(listed('n')).toMatchObject({ renamedByPerson: true, groupId: 'g' })
    expect(toldOn(IPC.SESSION_UPDATED).at(-1)).toMatchObject({ id: 'n', displayName: 'mine' })
    expect(events.find((e) => e.name === 'session-renamed')?.args).toEqual(['n', 'mine'])

    // An order a client set.
    note({
      op: 'upsert',
      kind: 'terminal',
      record: record('m', { pid: 0 }),
      native: true,
      created: true
    })
    await until('the second terminal', () => listed('m') !== undefined)
    note({ op: 'order', order: ['m', 'n'], native: true, reordered: true })
    await until('the order', () => toldOn(IPC.SESSION_REORDERED).length > 0)
    expect(ptyManager.getActiveSessions().map((s) => s.id)).toEqual(['m', 'n'])

    // The directories vornd holds while it prepares a session.
    note({ op: 'holds', holds: {}, nativeHolds: { '/w': 1 } })
    await until('the hold', () => isWorkspaceHeld('/w'))
    expect(isWorkspaceHeld('/held-at-start')).toBe(false)

    // A name set here, while vornd decides, is told to it as a patch.
    ptyManager.renameSession('n', 'ours')
    await until('the patch', () => fake.made('vornd:patch').length > 0)
    expect(fake.made('vornd:patch').at(-1)).toMatchObject({
      id: 'n',
      fields: { displayName: 'ours', renamedByPerson: true }
    })

    // Its worktree moved by vornd for a client: told as the server's own move is.
    const updates = toldOn(IPC.SESSION_UPDATED).length
    note({
      op: 'upsert',
      kind: 'terminal',
      record: record('n', {
        pid: 42,
        displayName: 'ours',
        renamedByPerson: true,
        groupId: 'g',
        branch: 'renamed',
        worktreePath: '/w/new-1a2b3c4d',
        worktreeName: 'new'
      }),
      native: true,
      moved: true
    })
    await until('the move', () => listed('n')?.worktreePath === '/w/new-1a2b3c4d')
    expect(listed('n')).toMatchObject({ branch: 'renamed', worktreeName: 'new' })
    expect(toldOn(IPC.SESSION_UPDATED).slice(updates)).toEqual([
      expect.objectContaining({ id: 'n', branch: 'renamed', worktreeName: 'new' })
    ])
    // A move the server made itself is told the same way.
    ptyManager.updateSessionsForWorktree('/w/new-1a2b3c4d', { branch: 'again' })
    expect(listed('n')?.branch).toBe('again')
    expect(toldOn(IPC.SESSION_UPDATED).at(-1)).toMatchObject({ id: 'n', branch: 'again' })

    // Closed by vornd: let go of here; its exit comes when the program ends.
    note({ op: 'remove', kind: 'terminal', id: 'n', native: true })
    await until('the close', () => listed('n') === undefined)
    expect(events.filter((e) => e.name === 'session-exit')).toHaveLength(1)
    expect(toldOn(IPC.TERMINAL_EXIT)).toEqual([])
    fake.send('vornd:effect', effect('n', 'exit', 9, { exitCode: 0 }))
    await until('its exit', () => toldOn(IPC.TERMINAL_EXIT).length === 1)
    expect(toldOn(IPC.TERMINAL_EXIT)[0]).toMatchObject({ id: 'n', exitCode: 0 })

    // One whose program could not be started ends as a failed spawn does.
    note({
      op: 'upsert',
      kind: 'terminal',
      record: record('m'),
      native: true,
      failed: 'no shell'
    })
    await until('the failure', () => toldOn(IPC.TERMINAL_EXIT).length === 2)
    expect(toldOn(IPC.TERMINAL_EXIT)[1]).toMatchObject({ id: 'm', exitCode: 1 })
    expect(listed('m')?.status).toBe('idle')

    // The last terminal in a worktree vornd closed: offered as a close here offers it.
    fake.send('vornd:cleanupOffer', { id: 'n', projectPath: '/p', worktreePath: '/w' })
    await until('the offer', () => toldOn(IPC.WORKTREE_CONFIRM_CLEANUP).length === 1)
    expect(toldOn(IPC.WORKTREE_CONFIRM_CLEANUP)[0]).toEqual({
      id: 'n',
      projectPath: '/p',
      worktreePath: '/w'
    })
  })

  it('claims conversations in vornd and tells it when the server winds down', async () => {
    expect(await vorndSessions.claim('conv', 's1')).toBeUndefined()
    fake.claimHolder = 'other'
    expect(await vorndSessions.claim('conv', 's2')).toBe('other')
    expect(fake.made('vornd:claim')).toEqual([
      { transcriptId: 'conv', sessionId: 's1' },
      { transcriptId: 'conv', sessionId: 's2' }
    ])
    vorndSessions.unclaim('s1', 'conv')
    vorndSessions.unclaim('s1')
    vorndSessions.preparing('s1')
    vorndSessions.prepared('s1')
    await until('the notes', () => fake.made('vornd:prepared').length === 1)
    expect(fake.made('vornd:unclaim')).toEqual([
      { sessionId: 's1', transcriptId: 'conv' },
      { sessionId: 's1' }
    ])
    expect(fake.made('vornd:preparing')).toEqual([{ sessionId: 's1' }])

    // Told on the subscription, then only when it changes.
    expect(fake.made('vornd:draining')).toEqual([{ draining: false, handingOver: false }])
    vorndSessions.tellClosing()
    closing = { draining: true, handingOver: false }
    vorndSessions.tellClosing()
    vorndSessions.tellClosing()
    await until('the change', () => fake.made('vornd:draining').length === 2)
    expect(fake.made('vornd:draining')[1]).toEqual({ draining: true, handingOver: false })
  })
})
