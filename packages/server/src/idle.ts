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

  /**
   * How long since anything used the hook endpoint.
   *
   * Separate from the client clock because it catches what nothing else does: an
   * agent the user started outside Vorn. Hooks are installed globally, so such a
   * run has no terminal, no headless entry and no socket here -- and `shutdown()`
   * uninstalls the hooks, which would break its permission routing mid-run.
   */
  msSinceHookActivity: number
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

  /**
   * Whether this server is bound wide, to be reached by something that is not
   * this machine.
   *
   * Read every tick rather than captured at boot: Network Access is an ordinary
   * toggle and the socket is rebound live, so a server can become -- or stop
   * being -- a phone's only way in at any point after it started.
   */
  servesOthers: boolean
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

/** Named so the verdict can tell this hold apart from the ones that mean work. */
const ATTACHED_DESKTOP = 'a desktop is attached'

export type IdleVerdict =
  | { exit: true }
  /**
   * `restartsCountdown` is not the same question as "is something holding it".
   *
   * Work restarts it: an agent that ran for three hours after the app closed
   * must get the full window afterwards, not none of it. Presence does not --
   * the attached-desktop flag is already gated on the client clock, and letting
   * it also reset the countdown would mean a slept laptop needs two windows to
   * release instead of the one that branch promises.
   */
  | { exit: false; because: string; restartsCountdown: boolean }

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
  // First, because it is the one hold that is about what this server is for
  // rather than what it is doing. Nothing restarts a server a phone reaches, and
  // "my phone could not reach my Mac this morning" is worse than a process that
  // outstays its welcome. Said out loud in Settings, since it makes the promise
  // there conditional.
  if (s.servesOthers) return 'this server is bound to be reached from the network'
  if (s.sessions > 0) return `${s.sessions} session(s)`
  // Over-conservative on purpose: `headless-manager` keeps exited entries for
  // thirty seconds, so this can read non-zero just after one finishes. Waiting
  // an extra half-minute is the cheap direction to be wrong in.
  if (s.headless > 0) return `${s.headless} headless agent(s)`
  // Presence, but never presence alone. Any authenticated socket may claim the
  // bridge and there is no heartbeat behind it, so a laptop that slept with Vorn
  // open leaves `isConnected` true for ever. An attached desktop that has said
  // nothing for the whole window is not attached, whatever the flag says -- and a
  // real one talks constantly, so this costs a live app nothing.
  //
  // Measured against the client clock rather than the caller's combined one:
  // that clock is the only one this veto cannot itself hold still. Anything
  // derived from "when the last hold let go" is reset by this very branch, so a
  // stuck bridge would keep the countdown at zero and never reach its own escape.
  if (s.bridgeAttached && s.msSinceClientActivity < policy.windowMs) return ATTACHED_DESKTOP
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
 * `quietForMs` is how long nothing has wanted this server, and the caller owns
 * it because it is the later of two clocks. One is when the last thing let go:
 * an agent that worked for three hours after the app closed must not leave the
 * server exiting on the very next tick with none of the grace the setting
 * implies. Another is when a hook last fired, which is the only trace an agent
 * started outside Vorn leaves here. The last is when a client spoke, and it is a
 * *duration* rather
 * than a count because MCP opens a fresh socket per RPC call -- the connected
 * count oscillates between zero and one while an agent works, and sampling it at
 * the wrong instant reads a busy server as an empty one. That same duration is
 * the only escape from the opposite failure: these sockets have no heartbeat, so
 * a half-open one would pin any count at one for ever.
 *
 * This is the whole decision. `IdleWatch` supplies the clocks and acts on the
 * answer; it does not get a second opinion.
 */
export function shouldExitWhenIdle(
  s: IdleSnapshot,
  policy: IdlePolicy,
  quietForMs: number
): IdleVerdict {
  const holding = whatHoldsItOpen(s, policy)
  if (holding) {
    return { exit: false, because: holding, restartsCountdown: holding !== ATTACHED_DESKTOP }
  }
  if (quietForMs < policy.windowMs) {
    return {
      exit: false,
      because: `quiet for only ${Math.round(quietForMs / 1000)}s`,
      restartsCountdown: false
    }
  }
  return { exit: true }
}

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
    // The most recent of the clocks wins. A client that spoke means somebody is
    // out there even with nothing running; a session that ended means the work
    // only just stopped; a hook that fired means an agent is mid-run somewhere
    // this server can otherwise not see.
    const sinceLetGo = this.quietSince === null ? 0 : Date.now() - this.quietSince
    const quietFor = Math.min(
      sinceLetGo,
      snapshot.msSinceClientActivity,
      snapshot.msSinceHookActivity
    )

    const verdict = shouldExitWhenIdle(snapshot, this.policy, quietFor)
    if (!verdict.exit) {
      // Busy restarts the clock; merely quiet-but-not-long-enough leaves it run.
      if (verdict.restartsCountdown) this.quietSince = null
      else this.quietSince ??= Date.now()
      return verdict
    }
    this.onIdle()
    return verdict
  }
}
