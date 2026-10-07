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
import { IPC, type AiAgentType, type HeadlessSession } from '../packages/shared/src/types'
import { headlessManager } from '../packages/server/src/headless-manager'
import { vorndSessions, type SessionNote } from '../packages/server/src/vornd-sessions'
import { FakeVornd, effect } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

/**
 * The server following the headless agents vornd starts and stops itself,
 * from the registry notes a vornd that starts them tells: an agent of every
 * kind taken on and listed, its program's start, its output read and told,
 * its exit told once after the last of the output and its record let go of,
 * and one whose program could not start. A fake vornd stands in, so each
 * note can be sent on its own.
 */

const GEN = 'g1'
const AGENTS: AiAgentType[] = ['claude', 'codex', 'copilot', 'gemini', 'opencode']

function record(id: string, extra: Partial<HeadlessSession> = {}): HeadlessSession {
  return {
    id,
    pid: 0,
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    isWorktree: false,
    status: 'running',
    startedAt: 1,
    launchCommand: '/opt/agents/bin/claude -p',
    ...extra
  }
}

describe('the headless agents vornd starts, followed here', () => {
  let dataDir: string
  let fake: FakeVornd
  let rev = 1
  const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
  const created: HeadlessSession[] = []
  const toldOn = (channel: string): Record<string, unknown>[] =>
    messages.filter((m) => m.channel === channel).map((m) => m.payload)
  const note = (fields: Partial<SessionNote> & { op: SessionNote['op'] }): void => {
    rev += 1
    fake.send('vornd:session', { gen: GEN, rev, ...fields })
  }
  const listed = (id: string): HeadlessSession | undefined =>
    headlessManager.getActiveSessions().find((s) => s.id === id)

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-headless-'))
    initDatabase(dataDir)
    fake = new FakeVornd(dataDir)
    fake.native = true
    fake.statuses = true
    fake.headless = true
    fake.state.registry = {
      gen: GEN,
      rev,
      terminals: [],
      headless: [],
      order: [],
      holds: {}
    }
    await fake.start()
    headlessManager.on('client-message', (channel: string, payload: Record<string, unknown>) =>
      messages.push({ channel, payload })
    )
    headlessManager.on('session-created', (session: HeadlessSession) => created.push(session))
    expect(await vorndSessions.connect(fake.endpoint)).toBe(true)
    expect(vorndSessions.createsHeadless()).toBe(true)
    expect(vorndSessions.createsTerminals()).toBe(false)
  })

  afterAll(async () => {
    vorndSessions.close()
    await fake.stop()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('takes on an agent of every kind vornd started, and reads what it prints', async () => {
    for (const agentType of AGENTS) {
      note({
        op: 'upsert',
        kind: 'headless',
        record: record(agentType, { agentType, rev: 9, exitAt: { epoch: 7, rseq: 1, index: 0 } }),
        native: true,
        created: true
      })
      await until(`${agentType} to be taken on`, () => listed(agentType) !== undefined)
      // The copy's revision and stamps are not the server's record.
      expect(listed(agentType)).toEqual(record(agentType, { agentType }))
      // Told once: a snapshot told again does not create it twice.
      note({
        op: 'upsert',
        kind: 'headless',
        record: record(agentType),
        native: true,
        created: true
      })
    }
    expect(created.map((s) => s.agentType)).toEqual(AGENTS)
    expect(headlessManager.getActiveSessions().map((s) => s.id)).toEqual(AGENTS)

    // Its program is up: read from the start.
    note({
      op: 'upsert',
      kind: 'headless',
      record: record('claude', { pid: 42 }),
      native: true,
      started: { pid: 42, epoch: 7 }
    })
    await until('the pid', () => listed('claude')?.pid === 42)
    await until('the output to be read', () =>
      fake.made('terminal:attach').some((p) => p.id === 'claude')
    )
    fake.sendOutput('claude', 7, 1, 'hello\nworld')
    await until('the output', () => toldOn(IPC.HEADLESS_DATA).length === 1)
    expect(toldOn(IPC.HEADLESS_DATA)[0]).toEqual({ id: 'claude', data: 'hello\nworld' })
    expect(headlessManager.getOutput('claude')).toEqual(['hello', 'world'])
  })

  it('tells the exit once, as vornd read it, after the last of the output', async () => {
    // vornd read the exit from the session: the record takes it at once.
    note({
      op: 'upsert',
      kind: 'headless',
      record: record('claude', { pid: 42, status: 'exited', exitCode: 3, endedAt: 1234 }),
      native: true
    })
    await until('the record to end', () => listed('claude')?.status === 'exited')
    expect(listed('claude')).toMatchObject({ exitCode: 3, endedAt: 1234 })
    // The exit itself waits for the output before it.
    expect(toldOn(IPC.HEADLESS_EXIT)).toEqual([])
    fake.send('vornd:effect', effect('claude', 'exit', 2, { exitCode: 3 }))
    fake.send('terminal:exit', { id: 'claude', exitCode: 3 })
    await until('the exit', () => toldOn(IPC.HEADLESS_EXIT).length === 1)
    expect(toldOn(IPC.HEADLESS_EXIT)[0]).toEqual({ id: 'claude', exitCode: 3 })
    // The same exit, told again: nothing moves.
    fake.send('vornd:effect', effect('claude', 'exit', 2, { exitCode: 3 }))
    await new Promise((r) => setTimeout(r, 50))
    expect(toldOn(IPC.HEADLESS_EXIT)).toHaveLength(1)
    expect(headlessManager.getActiveSessionsForWorktree('/p')).toEqual({ count: 0, sessionIds: [] })
  })

  it('ends an agent whose program could not be started as a failed spawn', async () => {
    note({
      op: 'upsert',
      kind: 'headless',
      record: record('gemini', { agentType: 'gemini', status: 'exited', exitCode: 1 }),
      native: true,
      failed: 'no such program'
    })
    await until('the failure', () => toldOn(IPC.HEADLESS_EXIT).length === 2)
    expect(toldOn(IPC.HEADLESS_EXIT)[1]).toEqual({ id: 'gemini', exitCode: 1 })
    expect(listed('gemini')).toMatchObject({ status: 'exited', exitCode: 1 })
  })

  it('asks vornd to stop an agent it follows, and lets go of them all on the way out', async () => {
    note({
      op: 'upsert',
      kind: 'headless',
      record: record('codex', { pid: 5 }),
      native: true,
      started: { pid: 5, epoch: 7 }
    })
    await until('codex to be up', () => listed('codex')?.pid === 5)
    headlessManager.killHeadless('codex')
    await until('the signal', () => fake.made('vornd:kill').length === 1)
    expect(fake.made('vornd:kill')[0]).toEqual({ id: 'codex', signal: 'term' })
    headlessManager.killAll()
    expect(headlessManager.getActiveSessions()).toEqual([])
    expect(vorndSessions.get('codex')).toBeUndefined()
  })
})
