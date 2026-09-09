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
 * What restarting for an update does to the sessions, in one line.
 *
 * It used to be a warning: the update stopped the server and ended every session.
 * The server is now handed over instead, so the line stays to say a promise is
 * kept -- somebody who read the old one needs telling it has changed.
 */
export function updateCostLine(sessionCount: number, aTurnIsRunning: boolean): string | null {
  if (sessionCount <= 0) return null
  const sessions =
    sessionCount === 1 ? 'Your session keeps running' : `Your ${sessionCount} sessions keep running`
  // The part people brace for, so it is named -- now to say it survives.
  const turn = aTurnIsRunning ? ' The turn in flight continues.' : ''
  return `${sessions} through the update.${turn}`
}
