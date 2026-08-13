import type { AgentStatus } from '../../shared/types'

/**
 * Only `waiting` takes the accent — it is the one status blocked on the person.
 * Running is the commonest state, so painting it would put bronzo across most
 * of the screen most of the time; it reads white and animates instead. Motion
 * is reserved for work actually in progress, so a waiting dot stays still and
 * lets the accent do the calling — the rule `workflow-status.ts` follows too.
 */
export const STATUS_DOT: Record<AgentStatus, string> = {
  running: 'bg-ink',
  waiting: 'bg-bronzo',
  idle: 'bg-ink-ghost',
  error: 'bg-danger'
}

export const STATUS_LABEL: Record<AgentStatus, string> = {
  running: 'Running',
  waiting: 'Waiting',
  idle: 'Idle',
  error: 'Error'
}

export const STATUS_TEXT: Record<AgentStatus, string> = {
  running: 'text-ink',
  waiting: 'text-bronzo',
  idle: 'text-ink-faint',
  error: 'text-danger'
}
