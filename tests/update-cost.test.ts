import { describe, it, expect } from 'vitest'
import { facesRestart, updateCostLine } from '../src/renderer/lib/update-cost'

describe('what restarting for an update costs', () => {
  it('says nothing when there is nothing to lose', () => {
    expect(updateCostLine(0, false)).toBeNull()
  })

  it('still says nothing when the count is nonsense', () => {
    expect(updateCostLine(-1, true)).toBeNull()
  })

  it('names one session without pluralising it', () => {
    expect(updateCostLine(1, false)).toBe('Your session restarts on the new version.')
  })

  it('counts them when there is more than one', () => {
    expect(updateCostLine(3, false)).toBe('Your 3 sessions restart on the new version.')
  })

  it('names the turn only when one is running', () => {
    // A session comes back where it was. A turn in flight does not, and that is
    // the only part actually lost — so it is said separately or not at all.
    expect(updateCostLine(3, true)).toBe(
      'Your 3 sessions restart on the new version. A turn in flight is lost.'
    )
    expect(updateCostLine(3, false)).not.toContain('turn')
  })
})

describe('which panes the update actually ends', () => {
  it('counts a session with something behind it', () => {
    expect(facesRestart({ ended: undefined })).toBe(true)
  })

  it('does not count one that already ended', () => {
    // Its card stays so the exit is readable, so it is still in `terminals` --
    // but it is already stopped, and the resume pass only takes back what it
    // stopped itself. Counting it promises an interruption that never comes.
    expect(facesRestart({ ended: { reason: 'app-closed', at: 1, replayed: true } })).toBe(false)
  })
})
