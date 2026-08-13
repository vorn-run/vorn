import type { AgentStatus } from '../../shared/types'

/**
 * Only `waiting` takes the accent — it is the one status blocked on the person.
 * Running is the commonest state, so painting it would put bronzo across most
 * of the screen most of the time; it reads white, and STATUS_GLYPH shimmers it.
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

export type StatusGlyph = 'shimmer' | 'circle-empty' | 'dot-solid'

// Only "running" shimmers — pulse is reserved for the one state where
// the agent is actively doing work. Waiting shows a static amber dot.
export const STATUS_GLYPH: Record<AgentStatus, StatusGlyph> = {
  running: 'shimmer',
  waiting: 'dot-solid',
  idle: 'circle-empty',
  error: 'dot-solid'
}

export const STATUS_TEXT: Record<AgentStatus, string> = {
  running: 'text-ink',
  waiting: 'text-bronzo',
  idle: 'text-ink-faint',
  error: 'text-danger'
}
