import type { UpdateStatus } from '../../shared/types'
import type { StatusTone } from './status-tone'

/**
 * What the UI needs to draw a status, derived from the raw updater state.
 *
 * This lives apart from the components because both surfaces — the Settings
 * panel and the sidebar banner — have to agree on what a status means, and
 * because the mapping is the part worth testing. The components then only lay
 * out what they are handed.
 */
export interface UpdateStatusView {
  /** One line, written for a person rather than named after the event. */
  label: string
  /**
   * The same fact in the width of a sidebar. Kept here rather than trimmed at
   * the call site so both surfaces stay in step when the wording changes.
   */
  shortLabel: string
  /** Supporting line: the channel, a timestamp, an error's detail. */
  detail: string | null
  /**
   * Which of the app's five status tones this state carries. Routed through the
   * shared vocabulary rather than naming colours here: status-tone.ts is the
   * only place that decides what a tone looks like, and a test enforces that
   * `blocked` keeps meaning exactly one thing.
   *
   * `blocked` is the accent, and only a staged update earns it — that is the
   * one state waiting on the person. Checking and downloading are the app's own
   * work, so they are `live`.
   */
  tone: StatusTone
  /** 0–100 while transferring, otherwise null. */
  percent: number | null
  /** The action worth offering here, if any. */
  action: 'restart' | 'download' | 'retry' | null
}

/** "2 hours ago", for a check that already happened. */
export function formatLastChecked(at: number | null, now: number = Date.now()): string | null {
  if (at == null) return null

  const seconds = Math.max(0, Math.round((now - at) / 1000))
  if (seconds < 60) return 'checked just now'

  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `checked ${minutes} minute${minutes === 1 ? '' : 's'} ago`

  const hours = Math.round(minutes / 60)
  if (hours < 24) return `checked ${hours} hour${hours === 1 ? '' : 's'} ago`

  const days = Math.round(hours / 24)
  return `checked ${days} day${days === 1 ? '' : 's'} ago`
}

export function describeUpdateStatus(
  status: UpdateStatus,
  channel: 'stable' | 'beta' = 'stable',
  now: number = Date.now()
): UpdateStatusView {
  switch (status.kind) {
    case 'idle':
      return {
        label: 'Up to date',
        shortLabel: 'Up to date',
        detail: formatLastChecked(status.lastCheckedAt, now) ?? `${channel} channel`,
        tone: 'idle',
        percent: null,
        action: null
      }

    case 'checking':
      return {
        label: 'Checking for updates…',
        shortLabel: 'Checking…',
        detail: `${channel} channel`,
        tone: 'live',
        percent: null,
        action: null
      }

    case 'available':
      // Only reachable with auto-download off; otherwise the updater moves
      // straight on to downloading.
      return {
        label: `Version ${status.version} is available`,
        shortLabel: `v${status.version} available`,
        detail: 'not downloaded yet',
        tone: 'idle',
        percent: null,
        action: 'download'
      }

    case 'downloading':
      return {
        label: status.version ? `Downloading ${status.version}` : 'Downloading update',
        shortLabel: `Downloading ${status.percent}%`,
        // The bar below carries the number; saying it twice in two places is
        // two things to keep in step for one fact.
        detail: null,
        tone: 'live',
        percent: status.percent,
        action: null
      }

    case 'ready':
      return {
        label: `Version ${status.version} is ready to install`,
        shortLabel: `v${status.version} ready`,
        detail: 'restart to apply',
        tone: 'blocked',
        percent: null,
        action: 'restart'
      }

    case 'error':
      return {
        label: "Couldn't check for updates",
        shortLabel: 'Update failed',
        detail: status.message,
        tone: 'broken',
        percent: null,
        action: 'retry'
      }

    case 'unsupported':
      return {
        label: 'Updates are off in development',
        shortLabel: 'Updates off',
        detail: 'packaged builds only',
        tone: 'idle',
        percent: null,
        action: null
      }
  }
}

/**
 * Whether anything should mark the sidebar. Deliberately not "is there a
 * status" — only a staged update earns a mark, because only that is waiting
 * on the person.
 */
export function hasPendingUpdate(status: UpdateStatus): boolean {
  return status.kind === 'ready'
}
