import { describe, it, expect } from 'vitest'
import { shouldOfferPane } from '../src/renderer/lib/pane-affordance'

/**
 * Which project panes a session is offered. A browser, a simulator and a panel
 * of shells belong to a session driving a project; beside a plain shell they
 * are three controls nobody there is going to reach for.
 */
describe('shouldOfferPane', () => {
  it('offers them to an agent session', () => {
    expect(shouldOfferPane('claude', false)).toBe(true)
    expect(shouldOfferPane('codex', false)).toBe(true)
  })

  it('withholds them from a shell', () => {
    expect(shouldOfferPane('shell', false)).toBe(false)
  })

  it('keeps the control for a pane a shell already has open', () => {
    // Hiding it would take away the only way to close what is on screen — the
    // same exception the device button has always made.
    expect(shouldOfferPane('shell', true)).toBe(true)
  })

  it('offers them while the session type is still unknown', () => {
    // A row that renders before its terminal record arrives should not flash
    // its controls in; erring towards showing matches every other gate here.
    expect(shouldOfferPane(undefined, false)).toBe(true)
  })
})
