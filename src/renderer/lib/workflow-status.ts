import type { NodeExecutionStatus } from '../../shared/types'
import { byTone, TONE_DOT, TONE_DOT_MOVING, TONE_TEXT, type StatusTone } from './status-tone'

/** `cancelled` is the only run-level state a node cannot be in. */
export type WorkflowStatusKey = NodeExecutionStatus | 'cancelled'

/** How a run ended, for the one-line verdict beside it in the list. */
export type RunOutcomeTone = 'success' | 'error' | 'waiting' | 'running' | 'neutral'

/**
 * What each step or run state means, in the shared vocabulary.
 *
 * This used to be four maps that disagreed: a run was a yellow dot but a blue
 * word, a success was a green dot but a grey outcome, and the sidebar kept a
 * fifth palette of its own. Reading them side by side was the only way to
 * notice, which is why they drifted.
 *
 * Only `waiting` is blocked on the person. Stopping a run is a decision rather
 * than a failure, so `cancelled` deliberately does not read as broken — that is
 * what leaves broken meaning "this broke". `status-tone.ts` decides what each of
 * these words looks like.
 */
export const WORKFLOW_STATUS_TONE: Record<WorkflowStatusKey, StatusTone> = {
  waiting: 'blocked',
  error: 'broken',
  running: 'live',
  success: 'settled',
  pending: 'idle',
  skipped: 'idle',
  cancelled: 'idle'
}

// Success earns the task board's done green: a finished step must read at a glance.
export const WORKFLOW_STATUS_DOT = {
  ...byTone(WORKFLOW_STATUS_TONE, TONE_DOT),
  success: 'bg-status-sage'
}

/**
 * The same dots, animated for the state that is actually moving.
 *
 * Kept apart from the static set because a filter chip and a stage segment are
 * legends, not live indicators — a pulsing key would suggest the *filter* is
 * doing something.
 */
export const WORKFLOW_STATUS_DOT_PULSE = {
  ...byTone(WORKFLOW_STATUS_TONE, TONE_DOT_MOVING),
  success: 'bg-status-sage'
}

export const WORKFLOW_STATUS_TEXT = byTone(WORKFLOW_STATUS_TONE, TONE_TEXT)

/**
 * A run's one-line verdict. Every tone is a status tone; `neutral` — a stopped
 * run, a run with nothing to say — reads as settled, which is what `success`
 * already means here.
 */
export function outcomeToneClass(tone: RunOutcomeTone): string {
  return WORKFLOW_STATUS_TEXT[tone === 'neutral' ? 'success' : tone]
}
