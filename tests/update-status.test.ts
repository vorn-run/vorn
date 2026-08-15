import { describe, it, expect } from 'vitest'
import {
  describeUpdateStatus,
  formatLastChecked,
  hasPendingUpdate
} from '../src/renderer/lib/update-status'

const NOW = 1_700_000_000_000

describe('formatLastChecked', () => {
  it('returns null when no check has happened', () => {
    expect(formatLastChecked(null, NOW)).toBeNull()
  })

  it('reads as "just now" under a minute', () => {
    expect(formatLastChecked(NOW - 30_000, NOW)).toBe('checked just now')
  })

  it('singularises the boundary units', () => {
    expect(formatLastChecked(NOW - 60_000, NOW)).toBe('checked 1 minute ago')
    expect(formatLastChecked(NOW - 3_600_000, NOW)).toBe('checked 1 hour ago')
    expect(formatLastChecked(NOW - 86_400_000, NOW)).toBe('checked 1 day ago')
  })

  it('pluralises beyond the boundary', () => {
    expect(formatLastChecked(NOW - 120_000, NOW)).toBe('checked 2 minutes ago')
    expect(formatLastChecked(NOW - 7_200_000, NOW)).toBe('checked 2 hours ago')
    expect(formatLastChecked(NOW - 172_800_000, NOW)).toBe('checked 2 days ago')
  })

  it('never reports the future as negative when clocks disagree', () => {
    expect(formatLastChecked(NOW + 5_000, NOW)).toBe('checked just now')
  })
})

describe('describeUpdateStatus', () => {
  it('spends the accent only on the state that is blocked on the person', () => {
    // status-tone.ts reserves `blocked` — the bronzo tone — for work waiting on
    // the user. A download in flight is the app's own work, so it must not
    // claim the accent.
    const tones = {
      idle: describeUpdateStatus({ kind: 'idle', lastCheckedAt: null }, 'stable', NOW).tone,
      checking: describeUpdateStatus({ kind: 'checking' }, 'stable', NOW).tone,
      available: describeUpdateStatus({ kind: 'available', version: '1.0.0' }, 'stable', NOW).tone,
      downloading: describeUpdateStatus(
        { kind: 'downloading', version: '1.0.0', percent: 40 },
        'stable',
        NOW
      ).tone,
      ready: describeUpdateStatus({ kind: 'ready', version: '1.0.0' }, 'stable', NOW).tone,
      error: describeUpdateStatus({ kind: 'error', message: 'boom' }, 'stable', NOW).tone,
      unsupported: describeUpdateStatus({ kind: 'unsupported' }, 'stable', NOW).tone
    }

    expect(Object.entries(tones).filter(([, tone]) => tone === 'blocked')).toEqual([
      ['ready', 'blocked']
    ])
    expect(tones.error).toBe('broken')
    expect(tones.checking).toBe('live')
    expect(tones.downloading).toBe('live')
  })

  it('falls back to the channel when nothing has been checked yet', () => {
    const view = describeUpdateStatus({ kind: 'idle', lastCheckedAt: null }, 'beta', NOW)
    expect(view.label).toBe('Up to date')
    expect(view.detail).toBe('beta channel')
    expect(view.action).toBeNull()
  })

  it('prefers the timestamp over the channel once a check has run', () => {
    const view = describeUpdateStatus(
      { kind: 'idle', lastCheckedAt: NOW - 7_200_000 },
      'stable',
      NOW
    )
    expect(view.detail).toBe('checked 2 hours ago')
  })

  it('offers download only when the transfer was deferred', () => {
    expect(
      describeUpdateStatus({ kind: 'available', version: '2.1.0' }, 'stable', NOW).action
    ).toBe('download')
    expect(
      describeUpdateStatus({ kind: 'downloading', version: '2.1.0', percent: 10 }, 'stable', NOW)
        .action
    ).toBeNull()
  })

  it('carries progress through for the bar, and only while downloading', () => {
    expect(
      describeUpdateStatus({ kind: 'downloading', version: '2.1.0', percent: 62 }, 'stable', NOW)
        .percent
    ).toBe(62)
    expect(
      describeUpdateStatus({ kind: 'ready', version: '2.1.0' }, 'stable', NOW).percent
    ).toBeNull()
  })

  it('stays sensible when the progress event arrives without a version', () => {
    // download-progress carries no version; an empty one must not render as
    // "Downloading " with a dangling space.
    const view = describeUpdateStatus(
      { kind: 'downloading', version: '', percent: 5 },
      'stable',
      NOW
    )
    expect(view.label).toBe('Downloading update')
  })

  it('surfaces the failure reason instead of swallowing it', () => {
    const view = describeUpdateStatus({ kind: 'error', message: 'ENOTFOUND' }, 'stable', NOW)
    expect(view.detail).toBe('ENOTFOUND')
    expect(view.action).toBe('retry')
  })

  it('explains the dev build rather than looking stuck', () => {
    const view = describeUpdateStatus({ kind: 'unsupported' }, 'stable', NOW)
    expect(view.label).toBe('Updates are off in development')
    expect(view.action).toBeNull()
  })

  it('gives every state a short label that fits a sidebar', () => {
    const statuses = [
      { kind: 'idle', lastCheckedAt: null },
      { kind: 'checking' },
      { kind: 'available', version: '1.2.3' },
      { kind: 'downloading', version: '1.2.3', percent: 50 },
      { kind: 'ready', version: '1.2.3' },
      { kind: 'error', message: 'a very long failure message that would never fit' },
      { kind: 'unsupported' }
    ] as const

    for (const status of statuses) {
      const { shortLabel } = describeUpdateStatus(status, 'stable', NOW)
      expect(shortLabel.length).toBeGreaterThan(0)
      expect(shortLabel.length).toBeLessThanOrEqual(22)
    }

    expect(
      describeUpdateStatus({ kind: 'ready', version: '1.2.3' }, 'stable', NOW).shortLabel
    ).toBe('v1.2.3 ready')
  })
})

describe('hasPendingUpdate', () => {
  it('marks the sidebar only for a staged update', () => {
    expect(hasPendingUpdate({ kind: 'ready', version: '1.0.0' })).toBe(true)
  })

  it('stays quiet for every other state, including work in flight', () => {
    expect(hasPendingUpdate({ kind: 'idle', lastCheckedAt: null })).toBe(false)
    expect(hasPendingUpdate({ kind: 'checking' })).toBe(false)
    expect(hasPendingUpdate({ kind: 'available', version: '1.0.0' })).toBe(false)
    expect(hasPendingUpdate({ kind: 'downloading', version: '1.0.0', percent: 99 })).toBe(false)
    expect(hasPendingUpdate({ kind: 'error', message: 'boom' })).toBe(false)
    expect(hasPendingUpdate({ kind: 'unsupported' })).toBe(false)
  })
})
