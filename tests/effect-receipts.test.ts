import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * The receiver's half of at-least-once effects: an effect id is acted on once,
 * by kind, and remembered in the database; with the database unwritable the
 * claim still holds for this process.
 */

const db = vi.hoisted(() => ({
  claimed: new Map<string, number>(),
  failing: false,
  pruned: [] as Array<[string, number]>
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/database', () => ({
  claimEffectReceipt: (id: string, kind: string, at: number) => {
    if (db.failing) throw new Error('database is locked')
    const key = `${kind}:${id}`
    if (db.claimed.has(key)) return false
    db.claimed.set(key, at)
    return true
  },
  pruneEffectReceipts: (kind: string, before: number) => {
    db.pruned.push([kind, before])
    return 0
  }
}))

import { claimEffect, resetEffectMemory } from '../packages/server/src/effect-receipts'

const DAY = 24 * 60 * 60 * 1000

beforeEach(() => {
  db.claimed.clear()
  db.failing = false
  db.pruned.length = 0
  resetEffectMemory()
})

describe('claimEffect', () => {
  it('is true once per id and kind', () => {
    expect(claimEffect('s:1:2:0', 'notify', 1)).toBe(true)
    expect(claimEffect('s:1:2:0', 'notify', 2)).toBe(false)
    expect(claimEffect('s:1:2:0', 'trigger', 3)).toBe(true)
  })

  it('remembers what the database already holds from an earlier run', () => {
    claimEffect('s:1:5:0', 'trigger', 1)
    resetEffectMemory()
    expect(claimEffect('s:1:5:0', 'trigger', 2)).toBe(false)
  })

  it('still stops repeats in this process when the database cannot be written', () => {
    db.failing = true
    expect(claimEffect('s:1:7:0', 'notify', 1)).toBe(true)
    expect(claimEffect('s:1:7:0', 'notify', 2)).toBe(false)
  })

  it('prunes notifications after a day and triggers after a week, at most hourly', () => {
    const now = 10 * DAY
    claimEffect('a', 'notify', now)
    claimEffect('b', 'notify', now + 1000)
    expect(db.pruned).toEqual([
      ['notify', now - DAY],
      ['trigger', now - 7 * DAY]
    ])
    // Forgotten in memory too once old enough, so the database decides again.
    db.failing = true
    claimEffect('c', 'notify', now + 2 * DAY)
    expect(claimEffect('a', 'notify', now + 2 * DAY)).toBe(true)
  })
})
