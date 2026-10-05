import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { initDatabase, closeDatabase, claimEffect } from '../packages/server/src/database'
import { FakeVornd, effect } from './helpers/fake-vornd'
import {
  VorndSessions,
  type HeldSession,
  type VorndExit
} from '../packages/server/src/vornd-sessions'
import {
  vorndSessionsAvailable,
  upstream,
  Vornd,
  BytesClient,
  until,
  home as testHome,
  killPid,
  announcedEndpoint
} from './helpers/vornd-sessions'

/**
 * The server's channel to vornd, and what it does with what vornd tells it.
 *
 * The first half talks to a stand-in for vornd, so a test can tell the server
 * the same effect twice, as vornd does after it restarts. The second goes
 * through the real binaries, when the conformance run built them.
 */

let dataDir: string

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-app-'))
  initDatabase(dataDir)
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('the server with a stand-in vornd', () => {
  let fake: FakeVornd
  let sessions: VorndSessions

  beforeEach(async () => {
    fake = new FakeVornd(dataDir)
    await fake.start()
    sessions = new VorndSessions()
  })

  afterEach(async () => {
    sessions.close()
    await fake.stop()
  })

  it('connects to vornd and starts sessions there by name', async () => {
    expect(sessions.inUse()).toBe(false)
    expect(await sessions.connect(fake.endpoint)).toBe(true)
    expect(sessions.inUse()).toBe(true)

    const pty = sessions.spawn('pane-1', { argv: ['sh'], cwd: '/', env: {} }, false)
    // Written before the spawn is answered: held until it is, then sent in order.
    pty.write('one')
    pty.write('two')
    await until('the spawn to be answered', () => pty.pid !== 0)
    await until(
      'the writes',
      () => fake.calls.filter((c) => c.method === 'terminal:write').length === 2
    )
    const spawn = fake.calls.find((c) => c.method === 'vornd:spawn')!
    expect(spawn.params.name).toBe('pane-1')
    expect(spawn.params.argv).toEqual(['sh'])
    const writes = fake.calls.filter((c) => c.method === 'terminal:write').map((c) => c.params.data)
    expect(writes).toEqual(['one', 'two'])
    expect(pty.epoch).toBe(7)

    pty.kill('SIGTERM')
    await until('the kill', () => fake.calls.some((c) => c.method === 'vornd:kill'))
    expect(fake.calls.find((c) => c.method === 'vornd:kill')!.params).toEqual({
      id: 'pane-1',
      signal: 'term'
    })
  })

  it('shows a notification once, however often vornd tells it (TP-T27)', async () => {
    const shown: string[] = []
    sessions.on('notify', (id: string, title: string) => shown.push(`${id}:${title}`))
    const note = effect('pane-1', 'notify', 4, { title: 'Done', body: 'built' })
    fake.state.notices = [note]
    await sessions.connect(fake.endpoint)
    await until('the kept notification', () => shown.length === 1)

    // Told live as well, then again by a vornd that restarted before its
    // checkpoint (RC-T22 b): the receipt keeps it to one.
    fake.send('vornd:effect', note)
    fake.dropAll()
    await sessions.connect(fake.endpoint)
    fake.send('vornd:effect', note)
    fake.send('vornd:effect', effect('pane-1', 'notify', 9, { title: 'Again', body: '' }))
    await until('the second notification', () => shown.length === 2)
    expect(shown).toEqual(['pane-1:Done', 'pane-1:Again'])
  })

  it('acts on an exit once: told again after a restart, it is only state (RC-T22 c)', async () => {
    await sessions.connect(fake.endpoint)
    const first = sessions.spawn(
      'agent-1',
      { argv: ['true'], cwd: '/', env: {}, piped: true },
      false
    )
    const exits: VorndExit[] = []
    first.onExit((e) => exits.push(e))
    await until('the spawn', () => first.pid !== 0)
    const exit = effect('agent-1', 'exit', 12, { code: 3, exitCode: 3 })
    fake.send('vornd:effect', exit)
    await until('the exit', () => exits.length === 1)
    expect(exits[0]).toEqual({ exitCode: 3, repeated: false })
    expect(first.isEnded).toBe(true)

    // The workflow step ran. vornd restarts and replays the records past its
    // checkpoint, so the same exit comes again to a session taken on afresh.
    const again = sessions.adopt(
      { id: 'agent-1', kind: 'piped', pid: 5, status: null, cwd: null, exit: null },
      false
    )
    const later: VorndExit[] = []
    again.onExit((e) => later.push(e))
    fake.send('vornd:effect', exit)
    await until('the replayed exit', () => later.length === 1)
    expect(later[0]).toEqual({ exitCode: 3, repeated: true })
    // Exactly one receipt for it.
    expect(claimEffect(exit.effectId, 'exit')).toBe(false)
  })

  it('takes states as states: status and cwd told twice change nothing more', async () => {
    await sessions.connect(fake.endpoint)
    const pty = sessions.spawn('pane-2', { argv: ['sh'], cwd: '/', env: {} }, false)
    const seen: string[] = []
    pty.on('status', (s: number) => seen.push(`status ${s}`))
    pty.on('cwd', (c: string) => seen.push(`cwd ${c}`))
    pty.on('activity', () => seen.push('activity'))
    await until('the spawn', () => pty.pid !== 0)
    fake.send('vornd:effect', effect('pane-2', 'status', 3, { status: 2 }))
    fake.send('vornd:effect', effect('pane-2', 'cwd', 4, { cwd: '/tmp' }))
    fake.send('vornd:activity', { id: 'pane-2' })
    await until('the effects', () => seen.length === 3)
    expect(seen).toEqual(['status 2', 'cwd /tmp', 'activity'])
  })

  it('ends a session vornd no longer holds once its holder says so, and keeps one it holds', async () => {
    await sessions.connect(fake.endpoint)
    const kept = sessions.spawn('kept', { argv: ['sh'], cwd: '/', env: {} }, false)
    const gone = sessions.spawn('gone', { argv: ['sh'], cwd: '/', env: {} }, false)
    await until('both spawns', () => kept.pid !== 0 && gone.pid !== 0)
    const exits: string[] = []
    kept.onExit(() => exits.push('kept'))
    gone.onExit((e) => exits.push(`gone ${e.exitCode}`))

    // vornd restarted without its holder: it cannot tell, so nothing ends.
    fake.dropAll()
    fake.state = { connected: false, sessions: [], ended: [], notices: [] }
    await sessions.connect(fake.endpoint)
    expect(exits).toEqual([])

    // Its holder is back, holding one of them and remembering how the other ended.
    const held = (id: string): HeldSession => ({
      id,
      kind: 'pty',
      pid: 9,
      status: null,
      cwd: null,
      exit: null
    })
    fake.state = {
      connected: true,
      sessions: [held('kept'), held('stranger')],
      ended: [{ ...held('gone'), exit: effect('gone', 'exit', 30, { code: 2, exitCode: 2 }) }],
      notices: []
    }
    const strangers: HeldSession[][] = []
    sessions.on('held', (list: HeldSession[]) => strangers.push(list))
    fake.send('vornd:connected', {})
    await until('the ended session', () => exits.length === 1)
    expect(exits).toEqual(['gone 2'])
    expect(kept.isEnded).toBe(false)
    // One it holds that nothing here stands for is offered to be taken on.
    await until('the strangers', () => strangers.length === 1)
    expect(strangers[0]!.map((s) => s.id)).toEqual(['stranger'])
  })

  it('reads a session from where it left off, and from a screen when it cannot continue', async () => {
    await sessions.connect(fake.endpoint)
    const pty = sessions.spawn('read-1', { argv: ['sh'], cwd: '/', env: {} }, true)
    let out = ''
    pty.onData((d) => (out += d))
    await until('the attach', () => fake.made('terminal:attach').length === 1)
    fake.sendOutput('read-1', 7, 0, 'a')
    // Another epoch's bytes are not this run's.
    fake.sendOutput('read-1', 8, 1, 'x')
    fake.sendOutput('read-1', 7, 1, 'b')
    await until('the bytes', () => out === 'ab')

    // vornd asks for a fresh read: from where this server stands.
    fake.send('terminal:resync', { id: 'read-1', reason: 'restarted' })
    await until('the second attach', () => fake.made('terminal:attach').length === 2)
    expect(fake.made('terminal:attach')[1]!.cursor).toEqual({
      epoch: 7,
      nextRseq: 2,
      nextOffset: 0
    })
    // A read that could not continue starts after the screen it was given.
    pty.attached({
      live: true,
      continued: false,
      cursor: { epoch: 9, nextRseq: 40, nextOffset: 0 }
    })
    fake.sendOutput('read-1', 9, 39, 'old')
    fake.sendOutput('read-1', 9, 40, 'c')
    await until('the new run', () => out === 'abc')

    // An attach answered with how it ended ends it.
    const exits: VorndExit[] = []
    pty.onExit((e) => exits.push(e))
    pty.attached({ live: false, exitCode: 6 })
    expect(exits).toEqual([{ exitCode: 6, repeated: false }])
    expect(pty.readCursor()).toEqual({ epoch: 9, nextRseq: 41, nextOffset: 0 })
  })

  it('waits for the last output before an exit, on a session it reads', async () => {
    await sessions.connect(fake.endpoint)
    const pty = sessions.spawn('read-2', { argv: ['sh'], cwd: '/', env: {} }, true)
    await until('the attach', () => fake.made('terminal:attach').length === 1)
    const seen: string[] = []
    pty.onData((d) => seen.push(d))
    pty.onExit((e) => seen.push(`exit ${e.exitCode}`))
    fake.send('vornd:effect', effect('read-2', 'exit', 5, { code: 2, exitCode: 2 }))
    fake.sendOutput('read-2', 7, 0, 'last words')
    await new Promise((r) => setTimeout(r, 50))
    expect(seen).toEqual(['last words'])
    fake.send('terminal:exit', { id: 'read-2', exitCode: 2 })
    await until('the exit', () => seen.length === 2)
    expect(seen).toEqual(['last words', 'exit 2'])
  })

  it('asks nothing of a vornd that is gone', async () => {
    expect(await sessions.readOutput('none')).toEqual([])
    await sessions.connect(fake.endpoint)
    fake.output = ['one', 'two']
    expect(await sessions.readOutput('x', 1)).toEqual(['one', 'two'])
    const pty = sessions.spawn('gone-1', { argv: ['sh'], cwd: '/', env: {} }, false)
    await until('the spawn', () => pty.pid !== 0)
    sessions.close()
    expect(sessions.inUse()).toBe(false)
    // Written, signalled and closed with nothing listening: dropped, not thrown.
    pty.write('lost')
    pty.kill('SIGINT')
    pty.closeStdin()
    expect(() => sessions.spawn('gone-2', { argv: ['sh'], cwd: '/', env: {} }, false)).toThrow()
  })

  it('refuses a vornd speaking another protocol', async () => {
    const other = new FakeVornd(dataDir)
    other.protocol = 99
    await other.start()
    expect(await sessions.connect(other.endpoint)).toBe(false)
    expect(await sessions.connect(path.join(dataDir, 'nothing.sock'))).toBe(false)
    await other.stop()
  })

  it('a spawn vornd refuses ends the session', async () => {
    await sessions.connect(fake.endpoint)
    await fake.stop()
    const pty = sessions.spawn('refused', { argv: ['sh'], cwd: '/', env: {} }, false)
    const exits: VorndExit[] = []
    pty.onExit((e) => exits.push(e))
    await until('the failed spawn', () => exits.length === 1)
    expect(exits[0]!.exitCode).toBe(1)
    fake = new FakeVornd(dataDir)
    await fake.start()
  })

  it('tells each refused start of a reused id, as a resume that fails twice does', async () => {
    await sessions.connect(fake.endpoint)
    fake.spawnError = 'no such program'
    const exits: VorndExit[] = []
    for (let n = 0; n < 2; n++) {
      const pty = sessions.spawn('resumed', { argv: ['nope'], cwd: '/', env: {} }, false)
      pty.onExit((e) => exits.push(e))
      await until(`refusal ${n + 1}`, () => exits.length === n + 1)
    }
    expect(exits).toEqual([
      { exitCode: 1, repeated: false },
      { exitCode: 1, repeated: false }
    ])
  })

  it('starts a session asked for while it connects, though vornd does not hold it yet', async () => {
    const connected = sessions.connect(fake.endpoint)
    const pty = sessions.spawn('during-connect', { argv: ['sh'], cwd: '/', env: {} }, false)
    const exits: VorndExit[] = []
    pty.onExit((e) => exits.push(e))
    expect(await connected).toBe(true)
    await until('the spawn to be answered', () => pty.pid !== 0)
    expect(fake.made('vornd:spawn')).toHaveLength(1)
    expect(pty.isEnded).toBe(false)
    expect(exits).toEqual([])
  })

  it('holds a signal sent before the start is answered until vornd knows the session', async () => {
    await sessions.connect(fake.endpoint)
    const pty = sessions.spawn('closed-at-once', { argv: ['sh'], cwd: '/', env: {} }, false)
    pty.kill('SIGHUP')
    await until('the kill', () => fake.made('vornd:kill').length === 1)
    const order = fake.calls.map((c) => c.method).filter((m) => m.startsWith('vornd:'))
    expect(order.indexOf('vornd:kill')).toBeGreaterThan(order.indexOf('vornd:spawn'))
    expect(fake.made('vornd:kill')[0]).toEqual({ id: 'closed-at-once', signal: 'hup' })
  })
})

describe.skipIf(!vorndSessionsAvailable)('the server with the real vornd', () => {
  let up: Awaited<ReturnType<typeof upstream>>
  let h: ReturnType<typeof testHome>
  let vornd: Vornd
  let sessions: VorndSessions

  beforeEach(async () => {
    up = await upstream()
    h = testHome()
    vornd = await Vornd.start(up.port, h.dir)
    sessions = new VorndSessions()
    await until('the announcement', () => announcedEndpoint(h.dir) !== null)
    expect(await sessions.connect(announcedEndpoint(h.dir)!)).toBe(true)
  })

  afterEach(async () => {
    sessions.close()
    const pid = await vornd.sessiondPid().catch(() => null)
    await vornd.kill()
    killPid(pid)
    up.close()
    h.remove()
  })

  it('runs a headless agent on pipes: prompt in, output read, exit once', async () => {
    const agent = sessions.spawn(
      'agent-1',
      { argv: ['sh', '-c', 'read line; echo "got $line"; exit 3'], cwd: '/', env: {}, piped: true },
      true
    )
    let out = ''
    agent.onData((d) => (out += d))
    const exits: VorndExit[] = []
    agent.onExit((e) => exits.push(e))
    agent.write('hello\n')
    agent.closeStdin()
    await until('the exit', () => exits.length === 1)
    expect(out).toContain('got hello')
    expect(exits[0]!.exitCode).toBe(3)
    expect(exits[0]!.repeated).toBe(false)
  })

  it('names the session with the server id, which clients attach by', async () => {
    const pty = sessions.spawn(
      'pane-a',
      {
        argv: ['sh', '-c', 'echo ready-$((40+2)); exec cat'],
        cwd: '/',
        env: {},
        cols: 80,
        rows: 24
      },
      false
    )
    await until('the spawn', () => pty.pid !== 0)
    const client = new BytesClient()
    await client.connect(vornd.port)
    await client.attach('pane-a')
    await until('the output', async () => (await client.text()).includes('ready-42'))
    // The same name cannot start a second session.
    const twin = sessions.spawn('pane-a', { argv: ['sh'], cwd: '/', env: {} }, false)
    const refused: VorndExit[] = []
    twin.onExit((e) => refused.push(e))
    await until('the refusal', () => refused.length === 1)
    expect(await sessions.readOutput('pane-a')).toEqual(expect.arrayContaining(['ready-42']))
    client.close()
  })

  it('keeps every session when vornd dies, and takes stock again when it is back (RC-T1)', async () => {
    const pty = sessions.spawn(
      'pane-b',
      { argv: ['sh', '-c', 'exec cat'], cwd: '/', env: {} },
      false
    )
    await until('the spawn', () => pty.pid !== 0)
    const ended: VorndExit[] = []
    pty.onExit((e) => ended.push(e))
    const shown: string[] = []
    sessions.on('notify', (_id: string, _title: string, body: string) => shown.push(body))
    // A notification, then vornd killed before anything else happens to it (RC-T22 b).
    pty.write('\x1b]9;from-cat\x07\n')
    await until('the notification', () => shown.length === 1)
    await vornd.kill()

    vornd = await Vornd.start(up.port, h.dir)
    await until('the new announcement', () => {
      const e = announcedEndpoint(h.dir)
      return e !== null
    })
    expect(await sessions.connect(announcedEndpoint(h.dir)!)).toBe(true)
    // Still running, and still answering.
    expect(ended).toEqual([])
    const client = new BytesClient()
    await client.connect(vornd.port)
    await client.attach('pane-b')
    pty.write('after-restart\n')
    await until('the echo', async () => (await client.text()).includes('after-restart'))
    // The notification is not shown a second time.
    await new Promise((r) => setTimeout(r, 200))
    expect(shown).toEqual(['from-cat'])
    client.close()

    pty.kill('SIGKILL')
    await until('the exit', () => ended.length === 1)
  })
})
