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

/** What restarting for an update does to the sessions: handed over, they survive; otherwise they end. */
export function updateCostLine(
  sessionCount: number,
  aTurnIsRunning: boolean,
  handedOver: boolean
): string | null {
  if (sessionCount <= 0) return null
  const subject = sessionCount === 1 ? 'Your session' : `Your ${sessionCount} sessions`
  if (!handedOver) {
    const turn = aTurnIsRunning ? ' The turn in flight is cut short.' : ''
    return `${subject} end${sessionCount === 1 ? 's' : ''} with the update.${turn}`
  }
  // The part people brace for, so it is named -- to say it survives.
  const turn = aTurnIsRunning ? ' The turn in flight continues.' : ''
  return `${subject} keep${sessionCount === 1 ? 's' : ''} running through the update.${turn}`
}
