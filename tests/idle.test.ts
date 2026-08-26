import { describe, it, expect, vi } from 'vitest'
import {
  IdleWatch,
  shouldExitWhenIdle,
  whatHoldsItOpen,
  DEFAULT_IDLE_WINDOW_MS,
  type IdleSnapshot,
  type IdlePolicy
} from '../packages/server/src/idle'

/**
 * Deciding whether a server with nobody using it should stop.
 *
 * The two wrong answers cost differently, and these tests are written around
 * that. Exiting while something is live runs `killAll()` on terminals and
 * headless agents, so a wrong yes destroys work. Never exiting only leaves a
 * leftover server in the way. So the bar for "yes" is high and every one of
 * these checks that a live thing vetoes.
 */

const policy: IdlePolicy = { windowMs: DEFAULT_IDLE_WINDOW_MS, schedulesHoldOpen: true }

const quiet = (over: Partial<IdleSnapshot> = {}): IdleSnapshot => ({
  sessions: 0,
  headless: 0,
  msSinceClientActivity: DEFAULT_IDLE_WINDOW_MS + 1,
  msSinceHookActivity: DEFAULT_IDLE_WINDOW_MS + 1,
  servesOthers: false,
  bridgeAttached: false,
  pendingPermissions: 0,
  pendingPairings: 0,
  connectorLeases: 0,
  enabledSchedules: 0,
  ...over
})

/**
 * The window check, for a server that has been quiet the whole time nobody has
 * been talking to it. `shouldExitWhenIdle` takes the quiet duration from its
 * caller because the caller owns the later of two clocks; where the only clock
 * is the client one, the two are the same number.
 */
function decide(s: IdleSnapshot, policy: IdlePolicy) {
  return shouldExitWhenIdle(s, policy, Math.min(s.msSinceClientActivity, s.msSinceHookActivity))
}

describe('what keeps a server alive', () => {
  it('exits when everything really is empty', () => {
    expect(decide(quiet(), policy)).toEqual({ exit: true })
  })

  it.each([
    ['a terminal session', { sessions: 1 }],
    ['a running headless agent', { headless: 1 }],
    ['an agent waiting on a permission', { pendingPermissions: 1 }],
    ['a pairing in progress', { pendingPairings: 1 }],
    ['outstanding connector work', { connectorLeases: 1 }],
    ['an enabled schedule', { enabledSchedules: 1 }]
  ])('stays up for %s', (_label, over) => {
    expect(decide(quiet(over), policy).exit).toBe(false)
  })

  it('stays up for an attached desktop that is still talking', () => {
    const attached = quiet({ bridgeAttached: true, msSinceClientActivity: 1_000 })
    expect(decide(attached, policy)).toMatchObject({ exit: false, restartsCountdown: false })
    expect(whatHoldsItOpen(attached, policy)).toBe('a desktop is attached')
  })

  it('does not stay up for a desktop that has said nothing for the whole window', () => {
    // The flag is presence, and presence has no heartbeat behind it: a laptop
    // that slept with Vorn open leaves `isConnected` true for ever. Without this,
    // that is a server nothing can ever stop. A desktop that is genuinely there
    // is sending frames, so it never reaches this.
    const stale = quiet({ bridgeAttached: true, msSinceClientActivity: policy.windowMs })
    expect(whatHoldsItOpen(stale, policy)).toBeNull()
    expect(decide(stale, policy)).toEqual({ exit: true })
  })

  it('says what is holding it, so a log line can explain itself', () => {
    const verdict = decide(quiet({ sessions: 2 }), policy)
    expect(verdict).toMatchObject({ exit: false, because: '2 session(s)' })
  })
})

describe('the three ways this could be wrongly true', () => {
  it('counts a session whose status is idle', () => {
    // `pty-manager` marks a session idle five seconds after its agent stops
    // typing. That is a live terminal with a shell in it. The snapshot carries a
    // count precisely so no caller can be tempted to filter by status the way
    // `getActiveSessionsForWorktree` does for a different question.
    expect(whatHoldsItOpen(quiet({ sessions: 1 }), policy)).toBe('1 session(s)')
  })

  it('does not read a gap between MCP calls as idleness', () => {
    // MCP opens a fresh socket per RPC call, so a connected *count* oscillates
    // 0↔1 while an agent is working. A client that acted a second ago is not an
    // absent client, however many sockets happen to be open this instant.
    const between = quiet({ msSinceClientActivity: 1_000 })
    expect(decide(between, policy)).toMatchObject({ exit: false })
  })

  it('still exits when a socket lingers but nothing has happened on it', () => {
    // The opposite failure, and the reason this is a duration and not a
    // presence test: there is no heartbeat, so a slept laptop or a dropped NAT
    // flow leaves an entry in the client map for ever. An earlier version asked
    // whether anybody was connected and returned "yes" here — which made this
    // test pass against a snapshot the real registry could never produce.
    expect(decide(quiet({ msSinceClientActivity: 60 * 60_000 }), policy).exit).toBe(true)
  })
})

describe('the window itself', () => {
  it('waits rather than exiting the moment the last thing goes', () => {
    const justQuiet = quiet({ msSinceClientActivity: 5_000 })
    expect(decide(justQuiet, policy)).toMatchObject({
      exit: false,
      because: 'quiet for only 5s'
    })
  })

  it('exits exactly at the window, not a tick before', () => {
    expect(decide(quiet({ msSinceClientActivity: policy.windowMs - 1 }), policy).exit).toBe(false)
    expect(decide(quiet({ msSinceClientActivity: policy.windowMs }), policy).exit).toBe(true)
  })

  it('is quiet only by the clock, never by who is connected', () => {
    expect(decide(quiet({ msSinceClientActivity: 0 }), policy)).toMatchObject({
      exit: false,
      because: 'quiet for only 0s'
    })
  })
})

describe('the schedule carve-out', () => {
  it('can be turned off, and then a schedule stops vetoing', () => {
    // The count fed in is only the schedules this server can service alone —
    // connector polls. A renderer-executed trigger is not in it, because
    // staying awake for one would keep a promise by dropping the run.
    const off: IdlePolicy = { ...policy, schedulesHoldOpen: false }
    expect(decide(quiet({ enabledSchedules: 3 }), off).exit).toBe(true)
    expect(decide(quiet({ enabledSchedules: 3 }), policy).exit).toBe(false)
  })
})

describe('the watch that acts on the decision', () => {
  const busy = (): IdleSnapshot => quiet({ sessions: 1 })

  it('does not exit while something is running, however long it waits', () => {
    let exits = 0
    const watch = new IdleWatch(busy, policy, () => exits++, 1)
    for (let i = 0; i < 5; i++) watch.tick()
    expect(exits).toBe(0)
  })

  it('starts the clock when the last thing lets go, not when a client last spoke', () => {
    // The bug this pins: the window used to be measured purely from client
    // activity. Close the app at 09:00, leave an agent running until 12:00, and
    // the moment it finished the elapsed time was already three hours — so the
    // very next tick exited, with none of the grace the setting implies.
    let snapshot: IdleSnapshot = quiet({ sessions: 1, msSinceClientActivity: 3 * 60 * 60_000 })
    let exits = 0
    const watch = new IdleWatch(
      () => snapshot,
      policy,
      () => exits++,
      1
    )

    expect(watch.tick()).toMatchObject({ exit: false, because: '1 session(s)' })

    // The session ends. A client has not spoken for three hours, but the work
    // only just stopped, so the wait starts now.
    snapshot = quiet({ sessions: 0, msSinceClientActivity: 3 * 60 * 60_000 })
    expect(watch.tick()).toMatchObject({ exit: false })
    expect(exits).toBe(0)
  })

  it('restarts the clock if something starts again mid-wait', () => {
    let snapshot = quiet({ msSinceClientActivity: 0 })
    let exits = 0
    const short: IdlePolicy = { windowMs: 50, schedulesHoldOpen: true }
    const watch = new IdleWatch(
      () => snapshot,
      short,
      () => exits++,
      1
    )

    watch.tick() // quiet, clock starts
    snapshot = quiet({ sessions: 1, msSinceClientActivity: 0 })
    watch.tick() // busy again, clock cleared
    snapshot = quiet({ msSinceClientActivity: 0 })
    expect(watch.tick()).toMatchObject({ exit: false }) // waiting afresh
    expect(exits).toBe(0)
  })

  it('exits once both clocks have run out', async () => {
    let exits = 0
    const short: IdlePolicy = { windowMs: 30, schedulesHoldOpen: true }
    const watch = new IdleWatch(
      () => quiet({ msSinceClientActivity: 10_000 }),
      short,
      () => exits++,
      1
    )

    watch.tick()
    await new Promise((r) => setTimeout(r, 60))
    watch.tick()

    expect(exits).toBe(1)
  })

  it('start is idempotent, matching the scheduler it borrows the shape from', () => {
    const watch = new IdleWatch(busy, policy, () => {}, 10_000)
    watch.start()
    watch.start()
    watch.stop()
    expect(() => watch.stop()).not.toThrow()
  })
})

describe('what a refused server is allowed to tell us', () => {
  it.each([
    ['a plain count', 3, 3],
    ['zero', 0, 0],
    ['a negative', -1, null],
    ['a fraction', 1.5, null],
    ['nonsense', Number.NaN, null],
    ['absurdly large', 999_999, null],
    ['a string', '5', null],
    ['nothing at all', undefined, null]
  ])('coerces %s', async (_label, sessions, expected) => {
    // The number rides the pre-auth identity frame from a server this app has
    // just decided it cannot use, and it ends up in Vorn's own warning. So it is
    // coerced rather than trusted: anything odd becomes null and the window says
    // "Sessions" instead of a figure.
    const { sessionsFrom } = await import('../src/main/server/server-launcher')
    const identity = {
      dataDir: '/x',
      appVersion: '1',
      buildChannel: 'packaged' as const,
      pid: 1,
      ...(sessions === undefined ? {} : { sessions })
    }
    expect(sessionsFrom(identity as never)).toBe(expected)
  })

  it('treats no identity at all as no count', async () => {
    const { sessionsFrom } = await import('../src/main/server/server-launcher')
    expect(sessionsFrom(null)).toBeNull()
  })
})

describe('what the promise in Settings does not cover', () => {
  it('never exits while bound wide, because nothing would bring it back', () => {
    // A phone is the only client, and "my phone could not reach my Mac this
    // morning" is worse than a leftover process. Said out loud in Settings
    // rather than left as a silent exception to the sentence there.
    const wide = quiet({ servesOthers: true })
    expect(whatHoldsItOpen(wide, policy)).toBe(
      'this server is bound to be reached from the network'
    )
    expect(decide(wide, policy).exit).toBe(false)
  })

  it('reads that per snapshot, so rebinding mid-life is not missed', () => {
    // `checkAndRebind` moves this socket between loopback and 0.0.0.0 while the
    // process runs. A watch that captured the boot host would arm itself on a
    // server that has since become a phone's only way in -- and, the other way
    // round, never release one that is now local-only.
    vi.useFakeTimers()
    try {
      let snapshot = quiet({ servesOthers: true })
      let exits = 0
      const watch = new IdleWatch(
        () => snapshot,
        policy,
        () => exits++,
        1
      )

      watch.tick()
      vi.advanceTimersByTime(policy.windowMs * 2)
      expect(watch.tick().exit).toBe(false)
      expect(exits).toBe(0)

      // Network Access turned off; the socket is rebound to loopback.
      snapshot = quiet({ servesOthers: false })
      watch.tick()
      vi.advanceTimersByTime(policy.windowMs)
      expect(watch.tick()).toEqual({ exit: true })
      expect(exits).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('an agent running outside Vorn', () => {
  it('holds the server open through the hook endpoint alone', () => {
    // Hooks are installed into the user's agent settings globally, so a `claude`
    // run the user started in their own terminal has no session record, no
    // headless entry and no socket here. `shutdown()` uninstalls those hooks, so
    // exiting under it breaks that run's permission routing for the rest of it.
    const outside = quiet({ msSinceHookActivity: 1_000 })
    expect(decide(outside, policy)).toMatchObject({ exit: false })
  })

  it('stops holding it once the hooks go quiet too', () => {
    expect(decide(quiet(), policy)).toEqual({ exit: true })
  })
})

describe('a desktop flag that outlived its desktop', () => {
  it('releases after one window, not two', () => {
    // The flag has no heartbeat behind it, so a slept laptop leaves it true for
    // ever. The escape is the client clock -- and the countdown must not be
    // restarted by this veto, or crossing the window would start a second one.
    vi.useFakeTimers()
    try {
      let ms = 0
      const snapshot = (): IdleSnapshot =>
        quiet({ bridgeAttached: true, msSinceClientActivity: ms, msSinceHookActivity: ms })
      let exits = 0
      const watch = new IdleWatch(snapshot, policy, () => exits++, 1)

      watch.tick()
      ms = policy.windowMs - 1
      vi.advanceTimersByTime(ms)
      expect(watch.tick()).toMatchObject({ exit: false, because: 'a desktop is attached' })
      expect(exits).toBe(0)

      // One millisecond later the veto lapses -- and the countdown it never
      // restarted has been running the whole time, so the server goes now
      // rather than at the end of a second window.
      ms = policy.windowMs
      vi.advanceTimersByTime(1)
      expect(watch.tick()).toEqual({ exit: true })
      expect(exits).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})
