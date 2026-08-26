import { beforeAll, afterAll } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/**
 * One spawner at a time, across test files.
 *
 * A handful of suites here start real detached servers, and vitest runs files in
 * parallel. Each server migrates a database, binds a port, claims a socket and
 * installs hooks, so several at once turn a machine's cores into the thing under
 * test: the failure that follows is a timeout in whichever suite was unluckiest,
 * usually not the one that caused it. `server-port-stability` was the one that
 * fell over when these were added.
 *
 * A lock file rather than vitest's own sequencing, because the tests that need
 * this are a named few and making the whole suite serial would cost every other
 * file minutes. Exclusive create is the same primitive `endpoint.ts` leans on,
 * for the same reason: it fails rather than overwriting, so the loser knows it
 * lost.
 *
 * A lock is stale when the worker that wrote it is gone, which is asked rather
 * than assumed from elapsed time: these suites legitimately run for minutes, and
 * a clock-based rule would hand the lock to a second worker exactly on the slow
 * machines where it matters. A worker killed mid-test still leaves one behind,
 * and that one is taken -- a run that refused to start over a crash an hour ago
 * would be worse than the contention this avoids.
 */

const LOCK = path.join(os.tmpdir(), 'vorn-test-spawn.lock')
const POLL_MS = 100

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

/** Who holds it, or null if nothing does. */
function holder(): number | null {
  try {
    const pid = Number(fs.readFileSync(LOCK, 'utf-8').trim())
    return Number.isInteger(pid) && pid > 0 ? pid : null
  } catch {
    return null
  }
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === 'EPERM'
  }
}

async function acquire(): Promise<void> {
  for (;;) {
    try {
      fs.writeFileSync(LOCK, String(process.pid), { flag: 'wx' })
      return
    } catch {
      // Liveness, never elapsed time. An earlier version called a lock stale
      // after two minutes, and `launcher-endpoint.process.test.ts` runs six tests
      // that each spawn a server -- comfortably longer than that. So the helper
      // written to stop suites running concurrently would have been the thing
      // letting a second one in, and only on the slowest machines, which is where
      // it was needed most. A worker that is still there still holds it, however
      // long it takes.
      const owner = holder()
      if (owner !== null && !alive(owner)) fs.rmSync(LOCK, { force: true })
      await sleep(POLL_MS)
    }
  }
}

/**
 * Hold the lock for this whole file.
 *
 * Per file rather than per test: the cost being avoided is servers running at
 * the same time, and a suite that released between its own tests would let
 * another file in halfway through its own sequence.
 */
export function spawnsRealServers(): void {
  beforeAll(async () => {
    await acquire()
  }, 180_000)
  afterAll(() => {
    // Only ours. The same rule the code under test lives by: no actor removes a
    // name it did not create. A worker that overran and lost the lock must not
    // take the next one's on its way out.
    if (holder() === process.pid) fs.rmSync(LOCK, { force: true })
  })
}
