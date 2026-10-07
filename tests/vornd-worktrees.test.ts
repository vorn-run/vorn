import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { FakeVornd } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'
import { VorndSessions } from '../packages/server/src/vornd-sessions'
import { linkWorktrees } from '../packages/server/src/vornd-worktrees'

describe('vornd cleaning worktrees', () => {
  let dataDir: string
  let fake: FakeVornd
  let sessions: VorndSessions
  const forgetSize = vi.fn()

  beforeEach(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-worktrees-'))
    fake = new FakeVornd(dataDir)
    await fake.start()
    sessions = new VorndSessions()
    forgetSize.mockClear()
  })

  afterEach(async () => {
    sessions.close()
    await fake.stop()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('forgets the sizes of the worktrees vornd cleaned, and nothing else', async () => {
    linkWorktrees({ channel: sessions, forgetSize })
    await sessions.connect(fake.endpoint)
    fake.send('vornd:worktreesCleaned', { paths: 'no' })
    fake.send('vornd:worktreesCleaned', null)
    fake.send('vornd:cleanupOffer', { id: 'n', projectPath: '/p', worktreePath: '/w' })
    fake.send('vornd:worktreesCleaned', { paths: ['/w/a', 7, '/w/b'] })
    await until('the sizes forgotten', () => forgetSize.mock.calls.length === 2)
    expect(forgetSize.mock.calls).toEqual([['/w/a'], ['/w/b']])
  })
})
