import type { TerminalState } from '../stores/types'

/**
 * Whether a pane has anything for the update to end.
 *
 * A session that has already ended keeps its card so the exit stays readable, so
 * `terminals` holds panes with no process behind them. They are not restarted --
 * they are already stopped, and the resume pass only takes back what it stopped
 * itself -- so counting them promises an interruption that is not going to
 * happen.
 */
export function facesRestart(terminal: Pick<TerminalState, 'ended'>): boolean {
  return terminal.ended === undefined
}

/** What restarting for an update does to the sessions, in one line. */
export function updateCostLine(
  sessionCount: number,
  aTurnIsRunning: boolean,
  sessionsSurvive: boolean
): string | null {
  if (sessionCount <= 0) return null
  const one = sessionCount === 1
  const subject = one ? 'Your session' : `Your ${sessionCount} sessions`
  const s = one ? 's' : ''
  const fate = sessionsSurvive ? `keep${s} running through the update` : `end${s} with the update`
  const turn = !aTurnIsRunning
    ? ''
    : sessionsSurvive
      ? ' The turn in flight continues.'
      : ' The turn in flight is cut short.'
  return `${subject} ${fate}.${turn}`
}
