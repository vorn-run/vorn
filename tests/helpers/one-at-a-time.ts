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
 * Stale locks are taken rather than swept. A worker killed mid-test leaves one
 * behind, and a test run that refused to proceed because of a crash an hour ago
 * would be worse than the contention this avoids.
 */

const LOCK = path.join(os.tmpdir(), 'vorn-test-spawn.lock')
const STALE_AFTER_MS = 120_000
const POLL_MS = 100

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

async function acquire(): Promise<void> {
  for (;;) {
    try {
      fs.writeFileSync(LOCK, String(process.pid), { flag: 'wx' })
      return
    } catch {
      try {
        if (Date.now() - fs.statSync(LOCK).mtimeMs > STALE_AFTER_MS)
          fs.rmSync(LOCK, { force: true })
      } catch {
        // Somebody released it between the failed create and this look, which is
        // the outcome being waited for anyway.
      }
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
    fs.rmSync(LOCK, { force: true })
  })
}
