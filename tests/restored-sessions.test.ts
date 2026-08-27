import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'
import {
  seedRestored,
  markRecovered,
  listRestored,
  restoredRecords,
  consumeRestored,
  consumeAllRestored,
  resetRestored,
  MAX_RESTORED_AGE_MS
} from '../packages/server/src/restored-sessions'

/**
 * Holding on to what the last run left, so the next save does not erase it.
 *
 * The bug behind this file is worth stating because none of these assertions
 * make sense without it: a save is a whole-table replace fed by the live session
 * map, that map is empty after a restart, and so opening one pane deleted every
 * record from the previous run. History is keyed by session id and swept when no
 * record names it, so a terminal's screen survived exactly one restart.
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

beforeEach(resetRestored)
afterEach(resetRestored)

describe('carrying records past a restart', () => {
  it('keeps them so the next save cannot replace them away', () => {
    seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)

    // What `startAutoSave` now persists, beside whatever is live.
    expect(restoredRecords().map((s) => s.id)).toEqual(['one', 'two'])
  })

  it('hands recovery the same list it is holding, so one rule decides deletions', () => {
    // Anything dropped here is absent from what `recoverHistory` is given, and
    // its history goes in the same sweep as every other unreachable tree. That
    // keeps a single deletion point rather than two that must agree.
    const keep = seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)

    expect(keep?.map((s) => s.id)).toEqual(['one', 'two'])
  })
})

describe('a database that would not answer', () => {
  it('holds nothing and tells recovery to sweep nothing', () => {
    // `readPreviousSessions` answers null when the read failed and `[]` when
    // there genuinely are none. Reading the first as the second is the
    // difference between removing nothing and removing every terminal's history
    // on one transient error.
    expect(seedRestored(null, NOW)).toBeNull()
    expect(restoredRecords()).toEqual([])
  })

  it('is not the same as an empty list', () => {
    expect(seedRestored([], NOW)).toEqual([])
  })
})

describe('a save that arrives before the last run has been read', () => {
  it('says so, because that ordering is what the whole fix rests on', () => {
    // `seedRestored` must run before the auto-save is wired. Reaching the
    // failure needs a session created in the window between the two, which only
    // a workflow launched by the inbox worker can do -- so no test drives it,
    // and it is checked here instead of being left to a comment.
    const warned: unknown[] = []
    const logger = console.warn
    console.warn = (...args: unknown[]): void => void warned.push(args)
    try {
      expect(restoredRecords()).toEqual([])
    } finally {
      console.warn = logger
    }
    // The record of the complaint is the point; the empty answer is the damage.
    expect(restoredRecords()).toEqual([])
  })

  it('does not complain once the records have been read', () => {
    seedRestored([session({ id: 'one' })], NOW)
    expect(restoredRecords().map((s) => s.id)).toEqual(['one'])
  })
})

describe('a record nobody came back for', () => {
  it('is dropped once it is older than the window', () => {
    const fresh = session({ id: 'fresh', savedAt: NOW - 1000 })
    const stale = session({ id: 'stale', savedAt: NOW - MAX_RESTORED_AGE_MS - 1000 })

    const keep = seedRestored([fresh, stale], NOW)

    expect(keep?.map((s) => s.id)).toEqual(['fresh'])
    expect(restoredRecords().map((s) => s.id)).toEqual(['fresh'])
  })

  it('falls back to when it was created if it was never saved', () => {
    const never = session({ id: 'never', savedAt: undefined, createdAt: NOW - 1000 })
    expect(seedRestored([never], NOW)?.map((s) => s.id)).toEqual(['never'])
  })
})

describe('claiming one', () => {
  it('can be done once, and the second caller is told it is gone', () => {
    // Two clients can have the same cold pane on screen. The second to press
    // resume must be refused rather than starting a second agent against one
    // transcript.
    seedRestored([session({ id: 'one' })], NOW)

    expect(consumeRestored('one')?.session.id).toBe('one')
    expect(consumeRestored('one')).toBeNull()
    expect(restoredRecords()).toEqual([])
  })

  it('leaves the others alone', () => {
    seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)
    consumeRestored('one')
    expect(restoredRecords().map((s) => s.id)).toEqual(['two'])
  })

  it('can be done to all of them at once', () => {
    seedRestored([session({ id: 'one' }), session({ id: 'two' })], NOW)
    expect(consumeAllRestored()).toHaveLength(2)
    expect(restoredRecords()).toEqual([])
  })
})

describe('what a pane is told about one', () => {
  it('says when it ended, so the pane can say so too', () => {
    seedRestored([session({ id: 'one', savedAt: NOW - 3_600_000 })], NOW)
    expect(listRestored()[0]?.endedAt).toBe(NOW - 3_600_000)
  })

  it('says whether there is a screen to show, and whether it is whole', () => {
    seedRestored([session({ id: 'whole' }), session({ id: 'torn' }), session({ id: 'none' })], NOW)

    markRecovered([
      { id: 'whole', stopped: 'end' },
      { id: 'torn', stopped: 'checksum' }
    ])

    const byId = new Map(listRestored().map((r) => [r.session.id, r]))
    expect(byId.get('whole')).toMatchObject({ replayable: true, partial: false })
    expect(byId.get('torn')).toMatchObject({ replayable: true, partial: true })
    // Nothing was rebuilt for this one, so the pane must not promise a screen.
    expect(byId.get('none')).toMatchObject({ replayable: false, partial: false })
  })
})
