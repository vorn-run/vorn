import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import {
  formatRelativeTime,
  formatRunDuration,
  formatCompactDuration
} from '../packages/web/src/renderer/lib/format-time'

const NOW = new Date('2026-04-16T12:00:00Z').getTime()

beforeAll(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
})

afterAll(() => {
  vi.useRealTimers()
})

describe('formatRelativeTime', () => {
  it('returns "Just now" for times less than a minute ago', () => {
    expect(formatRelativeTime(new Date(NOW - 30_000).toISOString())).toBe('Just now')
  })

  it('returns minutes for times less than an hour ago', () => {
    expect(formatRelativeTime(new Date(NOW - 5 * 60_000).toISOString())).toBe('5m ago')
    expect(formatRelativeTime(new Date(NOW - 59 * 60_000).toISOString())).toBe('59m ago')
  })

  it('returns hours for times less than a day ago', () => {
    expect(formatRelativeTime(new Date(NOW - 3 * 3_600_000).toISOString())).toBe('3h ago')
    expect(formatRelativeTime(new Date(NOW - 23 * 3_600_000).toISOString())).toBe('23h ago')
  })

  it('returns days for times less than 30 days ago', () => {
    expect(formatRelativeTime(new Date(NOW - 5 * 86_400_000).toISOString())).toBe('5d ago')
    expect(formatRelativeTime(new Date(NOW - 29 * 86_400_000).toISOString())).toBe('29d ago')
  })

  it('returns formatted date for times older than 30 days', () => {
    const result = formatRelativeTime(new Date(NOW - 60 * 86_400_000).toISOString())
    expect(result).not.toContain('ago')
    expect(result.length).toBeGreaterThan(0)
  })

  it('returns "Unknown" for invalid ISO strings', () => {
    expect(formatRelativeTime('not-a-date')).toBe('Unknown')
    expect(formatRelativeTime('')).toBe('Unknown')
  })

  it('returns absolute timestamp for future dates', () => {
    const future = new Date(NOW + 3_600_000).toISOString()
    const result = formatRelativeTime(future)
    expect(result).not.toBe('Just now')
    expect(result).not.toContain('ago')
  })
})

describe('formatRunDuration', () => {
  it('returns "running..." when end is undefined', () => {
    expect(formatRunDuration('2026-04-20T10:00:00Z')).toBe('running...')
  })

  it('formats sub-second durations as ms', () => {
    expect(formatRunDuration('2026-04-20T10:00:00.000Z', '2026-04-20T10:00:00.250Z')).toBe('250ms')
  })

  it('formats seconds with one decimal place under a minute', () => {
    expect(formatRunDuration('2026-04-20T10:00:00Z', '2026-04-20T10:00:01.400Z')).toBe('1.4s')
    expect(formatRunDuration('2026-04-20T10:00:00Z', '2026-04-20T10:00:45Z')).toBe('45.0s')
  })

  it('formats minutes and seconds at or above one minute', () => {
    expect(formatRunDuration('2026-04-20T10:00:00Z', '2026-04-20T10:02:13Z')).toBe('2m 13s')
    expect(formatRunDuration('2026-04-20T10:00:00Z', '2026-04-20T11:00:05Z')).toBe('60m 5s')
  })

  it('returns "—" for unparseable timestamps instead of "NaNms"', () => {
    expect(formatRunDuration('not-a-date', '2026-04-20T10:00:01Z')).toBe('—')
    expect(formatRunDuration('2026-04-20T10:00:00Z', 'still-not')).toBe('—')
  })

  it('returns "—" when end is before start instead of a negative duration', () => {
    expect(formatRunDuration('2026-04-20T10:00:05Z', '2026-04-20T10:00:00Z')).toBe('—')
  })
})

describe('formatCompactDuration', () => {
  it('returns "—" for negative durations', () => {
    expect(formatCompactDuration('2026-04-20T10:00:05Z', '2026-04-20T10:00:00Z')).toBe('—')
  })

  it('formats seconds-only durations as "Ns"', () => {
    expect(formatCompactDuration('2026-04-20T10:00:00Z', '2026-04-20T10:00:30Z')).toBe('30s')
  })

  it('formats one-minute durations as "MM:SS"', () => {
    expect(formatCompactDuration('2026-04-20T10:00:00Z', '2026-04-20T10:01:08Z')).toBe('1:08')
  })

  it('zero-pads the seconds component', () => {
    expect(formatCompactDuration('2026-04-20T10:00:00Z', '2026-04-20T10:02:03Z')).toBe('2:03')
  })

  it('counts up to now when end is omitted', () => {
    const start = new Date(NOW - 75_000).toISOString()
    expect(formatCompactDuration(start)).toBe('1:15')
  })

  it('returns "—" for unparseable timestamps instead of "NaN:NaN"', () => {
    expect(formatCompactDuration('not-a-date')).toBe('—')
    expect(formatCompactDuration('not-a-date', '2026-04-20T10:00:00Z')).toBe('—')
    expect(formatCompactDuration('2026-04-20T10:00:00Z', 'still-not')).toBe('—')
  })
})
