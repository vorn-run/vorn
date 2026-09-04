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

/**
 * What restarting for an update will cost, in one line, or null when it costs
 * nothing worth saying.
 *
 * Installing an update ends the server and every session on it, which is a
 * deliberate reversal of what `Keep Sessions Running` promises: that closing
 * Vorn leaves the agents working. Updating is a more considered act than closing
 * a window, so the reversal is defensible — but a setting the person turned on
 * cannot be quietly overruled, so the exception is stated on the button that
 * makes it rather than left to be discovered.
 *
 * The turn is named separately because it is the only part that is actually
 * lost. A session comes back where it was; a turn in flight does not.
 */
export function updateCostLine(sessionCount: number, aTurnIsRunning: boolean): string | null {
  if (sessionCount <= 0) return null
  const sessions =
    sessionCount === 1 ? 'Your session restarts' : `Your ${sessionCount} sessions restart`
  const turn = aTurnIsRunning ? ' A turn in flight is lost.' : ''
  return `${sessions} on the new version.${turn}`
}
