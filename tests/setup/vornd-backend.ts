import { afterAll, beforeAll } from 'vitest'
import type { LoopbackVornd } from '../helpers/loopback-vornd'

/**
 * Runs a test file with the Native daemon switch on: every terminal and piped
 * agent the server starts goes through the vornd link, answered in-process by
 * `LoopbackVornd` over the node-pty and child_process the file itself mocks.
 *
 * Loaded only when `VORN_TEST_BACKEND=vornd` (`yarn test:vornd-backend`), and
 * installed in `beforeAll`, after the file's own mocks are in place, so the
 * loopback spawns through them.
 */
let loopback: LoopbackVornd | null = null

beforeAll(async () => {
  const { installLoopbackVornd } = await import('../helpers/loopback-vornd')
  loopback = installLoopbackVornd()
})

afterAll(() => {
  process.stderr.write(`loopback calls ${loopback?.calls.length} ${loopback?.sessions.size}\n`)
  loopback?.uninstall()
  loopback = null
})
