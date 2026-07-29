import type { NodeExecutionStatus } from '../../../shared/types'

/** Run statuses share the node palette; `cancelled` is run-only. */
export type StatusDotKey = NodeExecutionStatus | 'running' | 'cancelled'

export const STATUS_DOT_STATIC: Record<StatusDotKey, string> = {
  success: 'bg-green-400',
  error: 'bg-red-500',
  running: 'bg-yellow-400',
  pending: 'bg-gray-600',
  skipped: 'bg-gray-600',
  waiting: 'bg-amber-400',
  // Stopping a run is a decision, not a failure — grey keeps red for things
  // that actually broke.
  cancelled: 'bg-gray-500'
}

export const STATUS_DOT_CLASSES: Record<StatusDotKey, string> = {
  ...STATUS_DOT_STATIC,
  running: `${STATUS_DOT_STATIC.running} animate-pulse`,
  waiting: `${STATUS_DOT_STATIC.waiting} animate-pulse`
}
