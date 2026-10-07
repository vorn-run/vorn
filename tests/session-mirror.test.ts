import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import type { HeadlessSession, TerminalSession } from '../packages/shared/src/types'
import { initDatabase, closeDatabase } from '../packages/server/src/database'
import {
  SessionMirror,
  VorndSessions,
  type RegistrySnapshot,
  type SessionNote
} from '../packages/server/src/vornd-sessions'
import { RecordFeed } from '../packages/server/src/session-feed'
import { FakeVornd } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

/**
 * vornd's copy of the session records, as the server follows it: changes in
 * revision order, and the whole copy again after a gap or a new vornd.
 */

function terminal(id: string, extra: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id,
    agentType: 'shell',
    projectName: 'p',
    projectPath: '/p',
    status: 'running',
    createdAt: 1,
    pid: 1,
    ...extra
  }
}

function headless(id: string, extra: Partial<HeadlessSession> = {}): HeadlessSession {
  return {
    id,
    pid: 2,
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    status: 'running',
    startedAt: 1,
    ...extra
  }
}

function snapshot(
  gen: string,
  rev: number,
  extra: Partial<RegistrySnapshot> = {}
): RegistrySnapshot {
  return { gen, rev, terminals: [], headless: [], order: [], holds: {}, ...extra }
}

function upsert(gen: string, rev: number, record: TerminalSession): SessionNote {
  return { gen, rev, op: 'upsert', kind: 'terminal', record: { ...record, rev } }
}

describe('SessionMirror', () => {
  let resyncs: number
  let mirror: SessionMirror

  beforeEach(() => {
    resyncs = 0
    mirror = new SessionMirror(() => resyncs++)
    mirror.load(snapshot('g1', 4, { terminals: [terminal('a', { rev: 3 })] }))
  })

  it('applies each change in revision order, and drops one it already has', () => {
    expect(mirror.apply(upsert('g1', 5, terminal('a', { displayName: 'one' })))).toBe('applied')
    expect(mirror.apply(upsert('g1', 6, terminal('b')))).toBe('applied')
    expect(mirror.apply({ gen: 'g1', rev: 7, op: 'order', order: ['b', 'a'] })).toBe('applied')
    expect(mirror.apply({ gen: 'g1', rev: 8, op: 'holds', holds: { '/w': 1 } })).toBe('applied')
    expect(
      mirror.apply({
        gen: 'g1',
        rev: 9,
        op: 'upsert',
        kind: 'headless',
        record: headless('h', { worktreePath: '/w' })
      })
    ).toBe('applied')
    // Told again, as after a reconnect: nothing moves.
    expect(mirror.apply(upsert('g1', 5, terminal('a', { displayName: 'stale' })))).toBe('stale')

    expect(mirror.terminals().map((t) => [t.id, t.displayName])).toEqual([
      ['b', undefined],
      ['a', 'one']
    ])
    expect(mirror.terminal('a')?.rev).toBe(5)
    expect(mirror.holds()).toEqual({ '/w': 1 })
    expect(mirror.activeInWorktree('/w')).toEqual({ count: 1, sessionIds: ['h'] })
    expect(mirror.revision).toEqual({ gen: 'g1', rev: 9 })

    expect(mirror.apply({ gen: 'g1', rev: 10, op: 'remove', kind: 'terminal', id: 'b' })).toBe(
      'applied'
    )
    expect(mirror.terminals().map((t) => t.id)).toEqual(['a'])
    expect(resyncs).toBe(0)
  })

  it('asks for the copy whole after a gap, and applies what came meanwhile on top', () => {
    // Revision 5 never came.
    expect(mirror.apply(upsert('g1', 6, terminal('b')))).toBe('resync')
    expect(resyncs).toBe(1)
    expect(mirror.apply(upsert('g1', 7, terminal('c')))).toBe('waiting')
    expect(mirror.apply(upsert('g1', 8, terminal('d')))).toBe('waiting')

    // The copy stands at 7: 6 and 7 are in it, 8 is applied on top.
    mirror.load(snapshot('g1', 7, { terminals: [terminal('a'), terminal('b'), terminal('c')] }))
    expect(mirror.terminals().map((t) => t.id)).toEqual(['a', 'b', 'c', 'd'])
    expect(mirror.revision).toEqual({ gen: 'g1', rev: 8 })
    expect(resyncs).toBe(1)
  })

  it('asks for the copy whole when vornd started again, whatever the revision', () => {
    expect(mirror.apply(upsert('g2', 1, terminal('x')))).toBe('resync')
    expect(resyncs).toBe(1)
    // The new vornd's copy, fed by the server since: nothing of the old one stays.
    mirror.load(snapshot('g2', 1, { terminals: [terminal('x')] }))
    expect(mirror.terminals().map((t) => t.id)).toEqual(['x'])
    expect(mirror.apply(upsert('g2', 2, terminal('y')))).toBe('applied')
    // A note of the old vornd that was still on its way.
    expect(mirror.apply(upsert('g1', 9, terminal('z')))).toBe('resync')
    expect(resyncs).toBe(2)
  })

  it('takes a snapshot the server sent as a change, in its revision order', () => {
    const note: SessionNote = {
      ...snapshot('g1', 5, { terminals: [terminal('s')], order: ['s'] }),
      op: 'snapshot'
    }
    expect(mirror.apply(note)).toBe('applied')
    expect(mirror.terminals().map((t) => t.id)).toEqual(['s'])
    expect(mirror.apply(note)).toBe('stale')
  })

  it('waits for its first copy before applying anything', () => {
    const fresh = new SessionMirror(() => resyncs++)
    expect(fresh.revision).toBeNull()
    expect(fresh.apply(upsert('g1', 2, terminal('late')))).toBe('waiting')
    fresh.load(snapshot('g1', 1))
    expect(fresh.terminals().map((t) => t.id)).toEqual(['late'])
    expect(resyncs).toBe(0)
  })

  it('tells the changes vornd made itself, and how each record reached it', () => {
    const took: Array<[string, string]> = []
    const native: SessionNote[] = []
    const own = new SessionMirror(
      () => resyncs++,
      (record, how) => took.push([record.id, how]),
      (note) => native.push(note)
    )
    own.load(snapshot('g1', 1, { terminals: [terminal('a')], nativeHolds: { '/w': 1 } }))
    // The holds the snapshot carried are told as a change, so they are acted on.
    expect(native.map((n) => [n.op, n.nativeHolds])).toEqual([['holds', { '/w': 1 }]])
    expect(own.nativeHolds()).toEqual({ '/w': 1 })

    // A record the server changed, one vornd renamed, one vornd created.
    own.apply(upsert('g1', 2, terminal('a', { cols: 90 })))
    own.apply({ ...upsert('g1', 3, terminal('a', { displayName: 'mine' })), native: true })
    own.apply({ ...upsert('g1', 4, terminal('n')), native: true, created: true })
    own.apply({
      ...upsert('g1', 5, terminal('n', { pid: 7 })),
      native: true,
      started: { pid: 7, epoch: 1 }
    })
    own.apply({ gen: 'g1', rev: 6, op: 'order', order: ['n', 'a'], native: true, reordered: true })
    own.apply({ gen: 'g1', rev: 7, op: 'holds', holds: {}, nativeHolds: {} })
    own.apply({ gen: 'g1', rev: 8, op: 'remove', kind: 'terminal', id: 'n', native: true })
    expect(took).toEqual([
      ['a', 'load'],
      ['a', 'note'],
      ['a', 'native'],
      ['n', 'note'],
      ['n', 'note']
    ])
    expect(native.slice(1).map((n) => [n.rev, n.op])).toEqual([
      [3, 'upsert'],
      [4, 'upsert'],
      [5, 'upsert'],
      [6, 'order'],
      [7, 'holds'],
      [8, 'remove']
    ])
    expect(own.nativeHolds()).toEqual({})
    expect(own.terminals().map((t) => t.id)).toEqual(['a'])
  })

  it('freezes its records, so changing one in place fails where it is written', () => {
    const record = mirror.terminal('a')!
    expect(() => {
      ;(record as { displayName?: string }).displayName = 'edited'
    }).toThrow(TypeError)
  })
})

describe('RecordFeed', () => {
  let sent: Array<Record<string, unknown>>
  let wants: boolean
  let feed: RecordFeed

  beforeEach(() => {
    sent = []
    wants = true
    feed = new RecordFeed()
    feed.attach({ wants: () => wants, send: (p) => sent.push(p) })
  })

  it('sends a record only when it changed, and nothing while vornd does not ask', () => {
    const t = terminal('a')
    feed.terminal(t)
    feed.terminal(t)
    expect(sent).toHaveLength(1)
    t.displayName = 'renamed'
    feed.terminal(t)
    expect(sent.map((s) => s.op)).toEqual(['upsert', 'upsert'])
    feed.order(['a'])
    feed.order(['a'])
    expect(sent).toHaveLength(3)

    wants = false
    t.displayName = 'unheard'
    feed.terminal(t)
    feed.remove('terminal', 'a')
    feed.statusAt('a', { epoch: 1, rseq: 1, index: 0 })
    expect(sent).toHaveLength(3)
  })

  it('carries the effect that set a status, until something else sets it', () => {
    const t = terminal('a', { status: 'waiting' })
    feed.statusAt('a', { epoch: 2, rseq: 9, index: 1 })
    feed.terminal(t)
    expect(sent[0]).toMatchObject({ statusAt: { epoch: 2, rseq: 9, index: 1 } })
    t.status = 'idle'
    feed.statusAt('a', null)
    feed.terminal(t)
    expect(sent[1]).not.toHaveProperty('statusAt')
    // A stamp alone is a change worth telling: vornd compares by it.
    feed.statusAt('a', { epoch: 2, rseq: 10, index: 0 })
    feed.terminal(t)
    expect(sent).toHaveLength(3)
  })

  it('removes only what it told, and starts over with a snapshot', () => {
    feed.remove('headless', 'never-told')
    expect(sent).toEqual([])
    const done = headless('done', { status: 'exited', exitCode: 0 })
    feed.setTerminalSource({ terminals: () => [terminal('a')], order: () => ['a'] })
    feed.setHeadlessSource(() => [headless('h'), done])
    feed.snapshot()
    // An agent that exited is one whose program ended, as a terminal's is.
    expect(sent[0]).toMatchObject({ op: 'snapshot', order: ['a'], holds: {}, ended: ['done'] })
    // What the snapshot carried is not sent again.
    feed.terminal(terminal('a'))
    feed.order(['a'])
    feed.headless(done, true)
    expect(sent).toHaveLength(1)
    feed.remove('headless', 'h')
    expect(sent[1]).toEqual({ op: 'remove', kind: 'headless', id: 'h' })
    // Ended is a change in itself, told once.
    const h = headless('h2')
    feed.headless(h)
    feed.headless(h, true)
    feed.headless(h, true)
    expect(sent.slice(2)).toEqual([
      { op: 'upsert', kind: 'headless', record: h },
      { op: 'upsert', kind: 'headless', record: h, ended: true }
    ])
  })
})

describe('the session records through the channel', () => {
  let dataDir: string
  let fake: FakeVornd
  let sessions: VorndSessions
  let feed: RecordFeed

  beforeEach(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'session-mirror-'))
    initDatabase(dataDir)
    fake = new FakeVornd(dataDir)
    await fake.start()
    sessions = new VorndSessions()
    feed = new RecordFeed()
    feed.setTerminalSource({ terminals: () => [terminal('a')], order: () => ['a'] })
    sessions.feedRecords(feed)
  })

  afterEach(async () => {
    sessions.close()
    await fake.stop()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('feeds a vornd that asks, then follows its copy and resyncs after a gap', async () => {
    const copy = snapshot('g1', 1, { terminals: [terminal('a', { rev: 1 })], order: ['a'] })
    fake.state.registry = copy
    expect(await sessions.connect(fake.endpoint)).toBe(true)
    // Everything first, before the subscription that answers with the copy.
    const methods = fake.calls.map((c) => c.method)
    expect(methods.indexOf('vornd:record')).toBeLessThan(methods.indexOf('vornd:subscribe'))
    expect(fake.made('vornd:record')[0]).toMatchObject({ op: 'snapshot', order: ['a'] })
    expect(sessions.mirror.revision).toEqual({ gen: 'g1', rev: 1 })

    feed.terminal(terminal('b'))
    await until('the record', () => fake.made('vornd:record').length === 2)
    expect(fake.made('vornd:record')[1]).toMatchObject({ op: 'upsert', kind: 'terminal' })

    fake.send('vornd:session', upsert('g1', 2, terminal('b')))
    await until('the change', () => sessions.mirror.revision?.rev === 2)

    // Revision 3 is lost: the mirror asks for the copy whole.
    fake.registry = snapshot('g1', 4, {
      terminals: [terminal('a'), terminal('b'), terminal('c')],
      order: ['a']
    })
    fake.send('vornd:session', upsert('g1', 4, terminal('c')))
    await until('the resync', () => sessions.mirror.revision?.rev === 4)
    expect(fake.made('vornd:registry')).toHaveLength(1)
    expect(sessions.mirror.terminals().map((t) => t.id)).toEqual(['a', 'b', 'c'])
  })
})
