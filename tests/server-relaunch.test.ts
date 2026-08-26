import { describe, it, expect } from 'vitest'
import {
  attemptsAfterExit,
  decideRelaunch,
  HEALTHY_UPTIME_MS,
  RELAUNCH_DELAYS_MS
} from '../src/main/server/server-relaunch'

/**
 * Whether a server that has died should be replaced.
 *
 * It never was. `child.on('exit')` logged the code and stopped, in both the dev
 * and production branches, so one crash ended the session — the app kept running
 * with live terminals and a dead server, every call failing, until it was quit
 * and reopened.
 *
 * Only half the recovery was missing. `ServerBridge.scheduleReconnect` has
 * always retried every two seconds and never given up; the wall of ECONNREFUSED
 * in the log was the client half working perfectly against a server nobody was
 * bringing back.
 *
 * The launcher itself forks processes and talks to Electron, so it cannot be
 * unit-tested here. The decision can, and it is where the ways to be wrong are.
 */
describe('deciding whether to restart', () => {
  it('restarts immediately the first time', () => {
    expect(decideRelaunch({ deliberate: false, hostMode: false, attempts: 0 })).toEqual({
      relaunch: true,
      delayMs: 0
    })
  })

  it('waits longer each time', () => {
    const delays = RELAUNCH_DELAYS_MS.map(
      (_, attempts) => decideRelaunch({ deliberate: false, hostMode: false, attempts }).delayMs
    )

    expect(delays).toEqual([...RELAUNCH_DELAYS_MS])
    for (let i = 1; i < delays.length; i++) {
      expect(delays[i]).toBeGreaterThan(delays[i - 1] as number)
    }
  })

  it('gives up rather than spinning', () => {
    // A server that dies this reliably dies on startup. Retrying forever turns
    // one broken thing into a process spawned every few seconds for as long as
    // the app is open.
    const decision = decideRelaunch({
      deliberate: false,
      hostMode: false,
      attempts: RELAUNCH_DELAYS_MS.length
    })

    expect(decision.relaunch).toBe(false)
    expect(decision.reason).toContain('in a row')
  })

  it('does not restart a server that was asked to stop', () => {
    // Quitting the app kills the server on purpose. Starting another one while
    // the app is on its way out is the opposite of what was asked.
    expect(decideRelaunch({ deliberate: true, hostMode: false, attempts: 0 }).relaunch).toBe(false)
  })

  it('starts nothing when the app is pointed at another machine', () => {
    // Host mode has no server of its own. `stopServer` already refuses to shut
    // a host's server down for the same reason — one person closing a laptop
    // must not take it away from everyone else.
    const decision = decideRelaunch({ deliberate: false, hostMode: true, attempts: 0 })

    expect(decision.relaunch).toBe(false)
    expect(decision.reason).toContain('another host')
  })

  it('is refused for the right reason when more than one applies', () => {
    expect(decideRelaunch({ deliberate: true, hostMode: true, attempts: 99 }).reason).toContain(
      'asked to stop'
    )
  })
})

describe('spending and refilling the budget', () => {
  it('forgets past failures once a server has run properly', () => {
    // Otherwise the budget is spent once and never returns: a machine left open
    // for a week, with three unrelated crashes days apart, would refuse to
    // restart on the third.
    expect(attemptsAfterExit(3, HEALTHY_UPTIME_MS)).toBe(0)
    expect(attemptsAfterExit(3, HEALTHY_UPTIME_MS + 60_000)).toBe(0)
  })

  it('keeps counting when a server dies quickly', () => {
    // And resetting on connection alone would be worse — a server that starts,
    // connects and dies immediately would refill the budget every time and
    // relaunch forever, which is the loop the cap exists to stop.
    expect(attemptsAfterExit(3, 1_000)).toBe(3)
    expect(attemptsAfterExit(3, HEALTHY_UPTIME_MS - 1)).toBe(3)
  })
})
