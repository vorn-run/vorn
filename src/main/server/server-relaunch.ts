/**
 * Whether a server that has just exited should be started again.
 *
 * It never was. `child.on('exit')` logged the code, set `serverProcess = null`
 * and stopped there, in both the dev and production branches — so a server that
 * died took the session with it. That is not theoretical: one malformed hook
 * payload killed it, and Vorn sat with live terminals and a dead server for two
 * and a half minutes, every call failing, until the app was quit and reopened.
 *
 * The other half of the recovery has been there all along. `ServerBridge`
 * reconnects every two seconds and never gives up, which is what that wall of
 * ECONNREFUSED in the log is — the client half working perfectly against a
 * server that was never coming back.
 *
 * Kept apart from the launcher because a decision with four inputs and three
 * ways to be wrong should be readable on its own, and because the launcher
 * itself cannot be unit-tested: it forks processes and talks to Electron.
 */
export interface RelaunchDecision {
  relaunch: boolean
  /** Milliseconds to wait first. Zero for the first attempt. */
  delayMs: number
  /** Why not, when not — for the log line that explains a session ending. */
  reason?: string
}

/** Backoff, and the point at which trying again is just spinning. */
export const RELAUNCH_DELAYS_MS = [0, 1_000, 5_000, 15_000]

export function decideRelaunch(input: {
  /** `stopServer` ran, or the app is quitting. The exit was the point. */
  deliberate: boolean
  /** No server of our own — we are pointed at someone else's. */
  hostMode: boolean
  /** How many times we have already relaunched in this run. */
  attempts: number
}): RelaunchDecision {
  if (input.deliberate) {
    return { relaunch: false, delayMs: 0, reason: 'the server was asked to stop' }
  }
  // Host mode has no server of its own: `serverProcess` is null because another
  // machine is running it. Starting one here would be answering a question
  // nobody asked, and `stopServer` already refuses to shut that server down for
  // the same reason.
  if (input.hostMode) {
    return { relaunch: false, delayMs: 0, reason: 'this app is connected to another host' }
  }
  if (input.attempts >= RELAUNCH_DELAYS_MS.length) {
    // A server that dies this reliably dies on startup, and relaunching it
    // forever turns one broken thing into a process being spawned every few
    // seconds for as long as the app is open.
    return {
      relaunch: false,
      delayMs: 0,
      reason: `it failed ${input.attempts} times in a row`
    }
  }

  return { relaunch: true, delayMs: RELAUNCH_DELAYS_MS[input.attempts] as number }
}

/**
 * How long a server must have run before its death counts as a fresh problem.
 *
 * Without this the budget is spent once and never returns: a machine left open
 * for a week, with three unrelated crashes days apart, would refuse to restart
 * on the third. And resetting on connection alone is worse -- a server that
 * starts, connects and dies immediately would reset the budget every time and
 * relaunch forever, which is the loop the cap exists to stop.
 */
export const HEALTHY_UPTIME_MS = 60_000

export function attemptsAfterExit(attempts: number, uptimeMs: number): number {
  return uptimeMs >= HEALTHY_UPTIME_MS ? 0 : attempts
}
