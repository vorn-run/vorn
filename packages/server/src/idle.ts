/**
 * Whether a server with nobody using it should stop.
 *
 * The server outlives the app now, and nothing made it stop — so the setting
 * that promises "a background server stays running until every session ends"
 * described an intention rather than the software. It also mattered more than
 * tidiness: a server this app cannot use is refused rather than replaced, so a
 * leftover one can stop Vorn opening at all, and a leftover was permanent.
 *
 * The decision lives here, apart from the timer that acts on it, for the reason
 * `server-relaunch.ts` gives about its own: the wiring cannot be unit tested and
 * the decision can, and the decision is where the ways to be wrong are.
 *
 * Both ways are expensive, differently. Exiting while something is live kills
 * it — `shutdown()` calls `killAll()` on terminals and headless agents, so a
 * wrong yes destroys the user's work. Never exiting leaves the leftover problem
 * exactly where it was. So: prefer staying up, and only say yes to things that
 * are positively, currently empty.
 */

export interface IdleSnapshot {
  /** Live PTYs. A session whose *status* is 'idle' still counts — see below. */
  sessions: number
  /** Headless agents still running. */
  headless: number
  /**
   * Milliseconds since a client last sent anything.
   *
   * A duration, never "is one connected". These sockets have no heartbeat and
   * are removed only on close or error, so a laptop that sleeps or a NAT that
   * drops the flow leaves an entry behind for ever — and any test of presence
   * would then hold the server open for ever with it. Time since real traffic
   * escapes that, and it covers the opposite hole too: MCP opens a fresh socket
   * per RPC call, so a connected *count* is zero between two calls of a working
   * agent.
   */
  msSinceClientActivity: number
  /** A Vorn desktop is driving this server through the browser/device bridge. */
  bridgeAttached: boolean
  /** Agents blocked on a permission prompt. Somebody is mid-decision. */
  pendingPermissions: number
  /** Pairing flows part-way through. A human is looking at a code. */
  pendingPairings: number
  /** Connector inbox leases outstanding: work claimed and not finished. */
  connectorLeases: number
  /**
   * Armed schedules this server can act on alone — connector polls only.
   *
   * A recurring or one-off trigger is executed by a renderer, so staying awake
   * for one buys nothing: with no client the occurrence is emitted, the minute
   * lock is written, and the run is lost either way. A connector poll really
   * does its work here.
   */
  enabledSchedules: number
}

export interface IdlePolicy {
  /** How long everything must stay empty before exiting. */
  windowMs: number
  /**
   * Whether an armed schedule keeps the server alive.
   *
   * Only counts the schedules the server can actually service on its own — see
   * `enabledSchedules`. Holding open for a renderer-executed trigger would be
   * keeping a promise by dropping it: the run is lost with no client either way,
   * and the wait costs every connector user a server that never exits. When
   * server-side execution lands, more triggers earn this and the count widens.
   */
  schedulesHoldOpen: boolean
}

export const DEFAULT_IDLE_WINDOW_MS = 30 * 60 * 1000

export type IdleVerdict = { exit: true } | { exit: false; because: string }

/**
 * Is there anything at all to stay up for?
 *
 * Separate from the window check so the caller can distinguish "busy" from
 * "quiet but not for long enough" — the first disarms the countdown, the second
 * lets it keep running.
 */
export function whatHoldsItOpen(s: IdleSnapshot, policy: IdlePolicy): string | null {
  // Not filtered by session status. `pty-manager` marks a session 'idle' five
  // seconds after its agent stops typing; that is a live terminal with a shell
  // in it, and `getActiveSessionsForWorktree` filters them out for a different
  // question entirely (whether a worktree is safe to delete).
  if (s.sessions > 0) return `${s.sessions} session(s)`
  // Over-conservative on purpose: `headless-manager` keeps exited entries for
  // thirty seconds, so this can read non-zero just after one finishes. Waiting
  // an extra half-minute is the cheap direction to be wrong in.
  if (s.headless > 0) return `${s.headless} headless agent(s)`
  if (s.bridgeAttached) return 'a desktop is attached'
  if (s.pendingPermissions > 0) return 'an agent is waiting on a permission'
  if (s.pendingPairings > 0) return 'a pairing is in progress'
  if (s.connectorLeases > 0) return 'connector work is outstanding'
  if (policy.schedulesHoldOpen && s.enabledSchedules > 0) {
    return `${s.enabledSchedules} enabled schedule(s)`
  }
  return null
}

/**
 * Whether to stop now.
 *
 * The client test is a *timestamp*, not a count, because MCP opens a fresh
 * socket for every RPC call: the connected count oscillates between zero and one
 * while an agent is working, and sampling it at the wrong instant reads a busy
 * server as an empty one. The same timestamp covers the opposite failure — there
 * is no heartbeat on these sockets, so a half-open one would otherwise pin the
 * count at one for ever and the server would never exit.
 */
export function shouldExitWhenIdle(s: IdleSnapshot, policy: IdlePolicy): IdleVerdict {
  const holding = whatHoldsItOpen(s, policy)
  if (holding) return { exit: false, because: holding }
  if (s.msSinceClientActivity < policy.windowMs) {
    return {
      exit: false,
      because: `quiet for only ${Math.round(s.msSinceClientActivity / 1000)}s`
    }
  }
  return { exit: true }
}

/**
 * The countdown that acts on the decision above.
 *
 * Deliberately dumb: it polls the snapshot, and the only judgement it makes is
 * the one `shouldExitWhenIdle` hands back. Two rules it does enforce, because
 * both are about *when* rather than *whether*:
 *
 * The exit is explicit, never a matter of letting the event loop drain. The
 * scheduler's inbox interval is not unref'd, so this process would sit there
 * for ever regardless of how empty it is.
 *
 * And the snapshot is taken twice — once to decide, once immediately before
 * acting. `shutdown()` calls `killAll()` on terminals and headless agents, so a
 * countdown that expires in the same instant a session starts would destroy it.
 * The second look costs nothing and closes that window.
 */
export class IdleWatch {
  private timer: NodeJS.Timeout | null = null

  constructor(
    private readonly snapshot: () => IdleSnapshot,
    private readonly policy: IdlePolicy,
    private readonly onIdle: () => void,
    checkEveryMs?: number
  ) {
    // Derived from the window rather than fixed, so the lateness is always a
    // fraction of the wait instead of a flat minute: a half-hour window is
    // checked every minute, and a short one — which is how tests watch a process
    // actually leave — is checked often enough to be observable.
    this.checkEveryMs =
      checkEveryMs ?? Math.max(250, Math.min(60_000, Math.floor(policy.windowMs / 4)))
  }

  private readonly checkEveryMs: number

  /** Idempotent, following the shape `scheduler.startInboxWorker` already uses. */
  start(): void {
    if (this.timer) return
    this.timer = setInterval(() => this.tick(), this.checkEveryMs)
    // Nothing should stay alive merely because this is watching.
    this.timer.unref?.()
  }

  stop(): void {
    if (!this.timer) return
    clearInterval(this.timer)
    this.timer = null
  }

  /** Exposed for tests and for a caller that wants to check without waiting. */
  /** When the last thing holding this server open let go. Null while busy. */
  private quietSince: number | null = null

  tick(): IdleVerdict {
    const snapshot = this.snapshot()
    const holding = whatHoldsItOpen(snapshot, this.policy)
    if (holding) {
      // Busy again: the clock restarts from whenever this ends, not from
      // whenever a client last spoke. Otherwise an agent working for three hours
      // after the window closed would leave the server exiting on the very next
      // tick, with none of the grace the setting implies.
      this.quietSince = null
      return { exit: false, because: holding }
    }
    this.quietSince ??= Date.now()

    // Two clocks, and the later one wins. A client that spoke recently means
    // somebody is out there even with nothing running; a session that ended
    // recently means the work only just stopped.
    const quietFor = Math.min(Date.now() - this.quietSince, snapshot.msSinceClientActivity)
    if (quietFor < this.policy.windowMs) {
      return { exit: false, because: `quiet for only ${Math.round(quietFor / 1000)}s` }
    }

    this.onIdle()
    return { exit: true }
  }
}
