import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'
import {
  seedRestored,
  consumeRestored,
  consumeAllRestored,
  restoreHeld,
  restoredRecords,
  resetRestored
} from '../packages/server/src/restored-sessions'
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
  resetRestored()
})

afterEach(() => {
  vi.restoreAllMocks()
  resetRestored()
})

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

  /**
   * Resume rebuilds the session from a whitelist under the same id and saves it,
   * so a field the whitelist forgets is written back as null and lost for good.
   * The group is the value that has to reach the handler for it to carry it.
   */
  it('carries a group on the record the resume handler reads', () => {
    seedRestored([session({ id: 'one', groupId: 'g1' })], NOW)

    expect(restoredRecords()[0].groupId).toBe('g1')
    expect(consumeRestored('one')?.session.groupId).toBe('g1')
  })

  /** The payload is client-shaped, so membership is applied server-side instead. */
  it('is not something buildRestorePayload carries, by design', () => {
    const payload = buildRestorePayload(session({ id: 'one', groupId: 'g1' }), undefined)

    expect(payload).not.toHaveProperty('groupId')
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
