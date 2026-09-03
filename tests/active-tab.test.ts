import { describe, it, expect } from 'vitest'
import { chooseActiveTab } from '../src/renderer/lib/active-tab'

const asked = (...ids: string[]): Set<string> => new Set(ids)
/** Before any sync pass has answered. */
const NOT_ASKED = null

describe('keeping the active tab pointing at something', () => {
  it('leaves the restored tab alone while the sessions are still coming', () => {
    // The list is empty for a moment on every cold launch. Clearing here is how
    // the restored tab used to be lost before its session ever arrived.
    expect(chooseActiveTab('term-1', [], NOT_ASKED)).toBeUndefined()
  })

  it('leaves it alone when the server has the session but the board does not show it', () => {
    // Reopen off: the session is ended and the banner is offering it back.
    expect(chooseActiveTab('term-2', ['term-1'], asked('term-1', 'term-2'))).toBeUndefined()
  })

  it('resolves a child pane to the session it hangs off', () => {
    expect(chooseActiveTab('browser:term-2', ['term-1'], asked('term-1', 'term-2'))).toBeUndefined()
  })

  it('moves to the first tab when the one it names is not among them', () => {
    expect(chooseActiveTab('term-9', ['term-1', 'term-2'], asked('term-1', 'term-2'))).toBe(
      'term-1'
    )
  })

  it('picks a tab when there is none active', () => {
    expect(chooseActiveTab(null, ['term-1'], asked('term-1'))).toBe('term-1')
  })

  it('clears it when there are no tabs at all', () => {
    expect(chooseActiveTab('term-9', [], asked('term-1'))).toBeNull()
  })

  it('says nothing when it is already clear and there is nothing to choose', () => {
    // Distinct from null: returning null here would set state on every render.
    expect(chooseActiveTab(null, [], asked())).toBeUndefined()
  })

  it('leaves a tab that is both known and shown exactly where it is', () => {
    expect(chooseActiveTab('term-1', ['term-1', 'term-2'], asked('term-1'))).toBeUndefined()
  })

  it('leaves a card alone when its owner is on the board', () => {
    const tabs = ['term-1', 'card:term-1:2']
    expect(chooseActiveTab('card:term-1:2', tabs, asked('term-1'))).toBeUndefined()
  })
})
