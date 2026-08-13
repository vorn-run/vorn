import type { NodeExecutionStatus } from '../../shared/types'
import type { RunOutcomeTone } from './run-presentation'

/**
 * How a workflow step or run reports its state, in one place.
 *
 * This used to be four maps that disagreed: a run was a yellow dot but a blue
 * word, a success was a green dot but a grey outcome, and the sidebar kept a
 * fifth palette of its own. Reading them side by side was the only way to
 * notice, which is why they drifted.
 *
 * The rule is the sessions surface's rule — see `status-colors.ts`. Only
 * `waiting` takes the accent, because only `waiting` is blocked on the person
 * and nothing else in a run needs them. Running is where a workflow spends most
 * of its life, so painting it would put bronzo across the canvas continuously;
 * it reads white and pulses instead. Everything settled recedes into the ink
 * ramp, `error` excepted — that is the one outcome worth an eye.
 */
export type WorkflowStatusKey = NodeExecutionStatus | 'running' | 'cancelled'

export const WORKFLOW_STATUS_DOT: Record<WorkflowStatusKey, string> = {
  waiting: 'bg-bronzo',
  error: 'bg-danger',
  running: 'bg-ink',
  success: 'bg-ink-faint',
  pending: 'bg-ink-ghost',
  skipped: 'bg-ink-ghost',
  // Stopping a run is a decision, not a failure — keeping it out of danger is
  // what leaves danger meaning "this broke".
  cancelled: 'bg-ink-ghost'
}

/**
 * The same dots, animated for the two states that are still moving.
 *
 * Kept apart from the static set because a filter chip and a stage segment are
 * legends, not live indicators — a pulsing key would suggest the *filter* is
 * doing something.
 */
export const WORKFLOW_STATUS_DOT_PULSE: Record<WorkflowStatusKey, string> = {
  ...WORKFLOW_STATUS_DOT,
  running: `${WORKFLOW_STATUS_DOT.running} animate-pulse`,
  waiting: `${WORKFLOW_STATUS_DOT.waiting} animate-pulse`
}

export const WORKFLOW_STATUS_TEXT: Record<WorkflowStatusKey, string> = {
  waiting: 'text-bronzo',
  error: 'text-danger',
  running: 'text-ink',
  success: 'text-ink-faint',
  pending: 'text-ink-ghost',
  skipped: 'text-ink-ghost',
  cancelled: 'text-ink-ghost'
}

/**
 * A run's one-line verdict. `neutral` covers the outcomes that carry no state
 * of their own — a stopped run, a run with nothing to say.
 */
export const WORKFLOW_OUTCOME_TEXT: Record<RunOutcomeTone, string> = {
  waiting: WORKFLOW_STATUS_TEXT.waiting,
  error: WORKFLOW_STATUS_TEXT.error,
  running: WORKFLOW_STATUS_TEXT.running,
  success: WORKFLOW_STATUS_TEXT.success,
  neutral: 'text-ink-faint'
}
