import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { TerminalSession } from '@vornrun/shared/types'
import {
  seedRestored,
  consumeRestored,
  consumeAllRestored,
  restoreHeld,
  restoredRecords,
  resetRestored
} from '../packages/server/src/restored-sessions'
import {
  configureHistory,
  startHistory,
  recordOutput,
  discardHistory,
  settleHistory,
  flushHistory,
  resetHistory
} from '../packages/server/src/history/writer'
import { historyDir } from '../packages/server/src/history/checkpoint'
import { createScreen, feedScreen, resetScreens } from '../packages/server/src/terminal-screen'
import {
  resetScrollback,
  seedScrollback,
  scrollbackUnitsHeld
} from '../packages/server/src/terminal-scrollback'
import { forgetRestored } from '../packages/server/src/register-methods'
import { buildRestorePayload } from '@vornrun/shared/session-restore'

/**
 * Taking a session from the last run, or letting it go.
 *
 * Both are the same decision made twice over: the record stops being offered and
 * what was written for it stops existing. The rule that matters is that it can
 * only happen once -- two panes can be looking at the same ended session, on two
 * devices, and the second to act must be told it is gone rather than starting a
 * second agent against one transcript.
 */

const NOW = 1_700_000_000_000
let dir: string

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'a-session',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'idle',
    createdAt: NOW - 60_000,
    pid: 4242,
    savedAt: NOW - 60_000,
    ...over
  } as TerminalSession
}

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-resume-'))
  resetRestored()
  resetScreens()
  resetScrollback()
  resetHistory()
  configureHistory(dir, { tickMs: 5, quiesceMs: 5_000, checkpointMs: 60_000 })
})

afterEach(() => {
  vi.restoreAllMocks()
  resetRestored()
  resetScreens()
  resetScrollback()
  resetHistory()
  fs.rmSync(dir, { recursive: true, force: true })
})

/** Put real files on disk for a session, the way a previous run would have. */
async function wrote(id: string): Promise<void> {
  createScreen(id, 80, 24)
  startHistory(id)
  feedScreen(id, 'output from the run before')
  recordOutput(id, 'output from the run before')
  await settleHistory()
}

describe('claiming one', () => {
  it('can be done once, and the second caller is told it is gone', () => {
    seedRestored([session({ id: 'one' })], NOW)

    expect(consumeRestored('one')?.session.id).toBe('one')
    // What the second pane, window or phone gets.
    expect(consumeRestored('one')).toBeNull()
  })

  it('stops the record being persisted, so it is not offered again', () => {
    seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)
    consumeRestored('one')

    expect(restoredRecords().map((s) => s.id)).toEqual(['two'])
  })
})

describe('what is on disk when a session is claimed or let go', () => {
  it('goes, because a live session opens its own rather than appending to it', async () => {
    await wrote('one')
    expect(fs.existsSync(historyDir(dir, 'one'))).toBe(true)

    await discardHistory('one')

    expect(fs.existsSync(historyDir(dir, 'one'))).toBe(false)
  })

  it('is refused while the server is shutting down', async () => {
    // `shutdown()` writes every terminal's screen and only then kills the PTYs,
    // and the teardown that follows runs the same paths a dismiss does. Without
    // this the last act of a clean shutdown is to delete what it just wrote.
    await wrote('one')
    await flushHistory()

    await discardHistory('one')

    expect(fs.existsSync(historyDir(dir, 'one'))).toBe(true)
  })

  it('is quiet about a session that never had any', async () => {
    await expect(discardHistory('never-recorded')).resolves.toBeUndefined()
  })
})

describe('letting all of them go at once', () => {
  it('takes every record, so nothing is left half-offered', () => {
    seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)

    expect(consumeAllRestored().map((r) => r.session.id)).toEqual(['one', 'two'])
    expect(restoredRecords()).toEqual([])
    expect(consumeRestored('one')).toBeNull()
  })
})

describe('turning a record back into a launch', () => {
  it('carries the worktree a session was running in', () => {
    const payload = buildRestorePayload(
      session({ isWorktree: true, worktreePath: '/dev/vorn-wt', branch: 'p4/restore' }),
      'agent-session-id'
    )

    expect(payload).toMatchObject({
      existingWorktreePath: '/dev/vorn-wt',
      branch: 'p4/restore',
      resumeSessionId: 'agent-session-id'
    })
  })

  it('refuses a shell, which has no resume to build', () => {
    // A shell restores by starting one in the directory it was in. Building an
    // agent launch line for it would produce a command nothing can run.
    expect(() => buildRestorePayload(session({ agentType: 'shell' }))).toThrow()
  })
})

describe('a claim whose spawn then fails', () => {
  it('puts the record back, because otherwise there is nothing to try again from', () => {
    // Claiming is destructive on purpose -- it is what stops two clients
    // starting two agents against one transcript. But a claim that then fails
    // to spawn would leave the session in neither place: gone from here, never
    // in the pty manager, and erased by the next save. Reachable without malice:
    // a project directory renamed, a worktree pruned, a volume unmounted.
    seedRestored([session({ id: 'one' })], NOW)
    const claimed = consumeRestored('one')
    expect(claimed).not.toBeNull()
    expect(restoredRecords()).toEqual([])

    restoreHeld(claimed!)

    expect(restoredRecords().map((s) => s.id)).toEqual(['one'])
    // And it can be claimed again, once.
    expect(consumeRestored('one')).not.toBeNull()
    expect(consumeRestored('one')).toBeNull()
  })
})

describe('letting go of everything held for one', () => {
  it('frees the scrollback recovery seeded, which nothing else would', async () => {
    // Recovery gives a restored session a scrollback so a pane can be shown its
    // last screen. That session has no PTY, so it never reaches the
    // `clearScrollback` on the kill path, and being claimed or dismissed is the
    // last thing that happens to it -- so without this the bytes are held for
    // the life of the server, once per session anyone declines.
    await wrote('one')
    seedScrollback('one', 'what the last run had on screen')
    expect(scrollbackUnitsHeld('one')).toBeGreaterThan(0)

    await forgetRestored('one')

    expect(scrollbackUnitsHeld('one')).toBe(0)
  })

  it('takes the history with it', async () => {
    await wrote('two')
    expect(fs.existsSync(historyDir(dir, 'two'))).toBe(true)

    await forgetRestored('two')

    expect(fs.existsSync(historyDir(dir, 'two'))).toBe(false)
  })
})
