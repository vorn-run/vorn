import type { NodeExecutionStatus } from '../../shared/types'

/** `cancelled` is the only run-level state a node cannot be in. */
export type WorkflowStatusKey = NodeExecutionStatus | 'cancelled'

/** How a run ended, for the one-line verdict beside it in the list. */
export type RunOutcomeTone = 'success' | 'error' | 'waiting' | 'running' | 'neutral'

/**
 * How a workflow step or run reports its state, in one place.
 *
 * This used to be four maps that disagreed: a run was a yellow dot but a blue
 * word, a success was a green dot but a grey outcome, and the sidebar kept a
 * fifth palette of its own. Reading them side by side was the only way to
 * notice, which is why they drifted.
 *
 * The rule is the sessions surface's rule — see `status-colors.ts`, in colour
 * and in motion both. Only `waiting` takes the accent, because only `waiting`
 * is blocked on the person and nothing else in a run needs them. Running is
 * where a workflow spends most of its life, so painting it would put bronzo
 * across the canvas continuously; it reads white instead. Everything settled
 * recedes into the ink ramp, `error` excepted — that is the one outcome worth
 * an eye.
 */
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
 * The same dots, animated for the state that is actually moving.
 *
 * Only `running`. A waiting gate wants attention, but it is not doing anything
 * — bronzo is what calls you to it, and motion stays a report of work in
 * progress. This is the rule `status-colors.ts` already follows for a waiting
 * session, and having the two disagree meant a gate pulsed in the run list and
 * sat still in the dock at the same moment.
 *
 * Kept apart from the static set because a filter chip and a stage segment are
 * legends, not live indicators — a pulsing key would suggest the *filter* is
 * doing something.
 */
export const WORKFLOW_STATUS_DOT_PULSE: Record<WorkflowStatusKey, string> = {
  ...WORKFLOW_STATUS_DOT,
  running: `${WORKFLOW_STATUS_DOT.running} animate-pulse`
}

/**
 * Spelled out rather than derived from the dots by swapping `bg-` for `text-`.
 * Tailwind scans source text for candidate class names, so a name built at
 * runtime is a name it never sees — deriving these would silently leave
 * `text-ink-ghost` ungenerated and the settled statuses unstyled.
 */
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
 * A run's one-line verdict. Every tone is a status tone; `neutral` — a stopped
 * run, a run with nothing to say — reads as settled, which is what `success`
 * already means here.
 */
export function outcomeToneClass(tone: RunOutcomeTone): string {
  return WORKFLOW_STATUS_TEXT[tone === 'neutral' ? 'success' : tone]
}
