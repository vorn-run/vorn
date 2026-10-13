/**
 * Workflow-run duration in the verbose RunEntry format
 * (e.g. "1.4s", "2m 13s"). Returns "running..." when `end` is absent and
 * "—" when either timestamp is unparseable or `end` is before `start`.
 */
export function formatRunDuration(start: string, end?: string): string {
  if (!end) return 'running...'
  const ms = new Date(end).getTime() - new Date(start).getTime()
  if (Number.isNaN(ms) || ms < 0) return '—'
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`
}

/** Compact "MM:SS" or "Ns" for table cells. Counts up to now when `end` is absent. */
export function formatCompactDuration(start: string, end?: string): string {
  const ms = (end ? new Date(end).getTime() : Date.now()) - new Date(start).getTime()
  if (Number.isNaN(ms) || ms < 0) return '—'
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s`
  const mins = Math.floor(ms / 60_000)
  const secs = Math.floor((ms % 60_000) / 1000)
  return `${mins}:${String(secs).padStart(2, '0')}`
}

export function formatRelativeTime(iso: string): string {
  const time = new Date(iso).getTime()
  if (Number.isNaN(time)) return 'Unknown'
  const diff = Date.now() - time
  if (diff < 0) return new Date(iso).toLocaleString()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'Just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  const days = Math.floor(hrs / 24)
  if (days < 30) return `${days}d ago`
  return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}
