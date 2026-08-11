import { describe, it, expect } from 'vitest'
import { humanCron } from '../src/renderer/lib/cron-text'

describe('humanCron', () => {
  it('reads a minute interval the way someone would say it', () => {
    expect(humanCron('*/5 * * * *')).toBe('every 5 minutes')
    expect(humanCron('*/15 * * * *')).toBe('every 15 minutes')
  })

  it('does not say "1 minutes"', () => {
    expect(humanCron('*/1 * * * *')).toBe('every 1 minute')
  })

  it('reads the every-minute form, which has no interval to name', () => {
    expect(humanCron('* * * * *')).toBe('every minute')
  })

  it('shows an expression it cannot read rather than guessing at it', () => {
    // A wrong reading of a schedule is worse than an unfamiliar one: someone
    // acting on "every 5 minutes" when it is really Mondays at nine has been
    // actively misled.
    expect(humanCron('0 9 * * 1')).toBe('0 9 * * 1')
    expect(humanCron('0 */2 * * *')).toBe('0 */2 * * *')
    expect(humanCron('')).toBe('')
  })
})
