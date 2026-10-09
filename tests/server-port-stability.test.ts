import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { builtVornd, startServed } from './helpers/served'

spawnsRealServers()

/**
 * The one assertion the unit tests cannot make: two launches agree.
 *
 * vornd's port choice is covered as a pure decision in its crate. This boots
 * the real binary twice on one data directory, asking for no port as the app
 * does: either the first launch bound the default and remembered it, or the
 * default was busy and it remembered the fallback. Both leave the second
 * launch on the same port. Its home is the test's own, as is its data.
 */
describe.skipIf(!builtVornd)('a server relaunched on the same data directory', () => {
  it('comes back on the port it used before', async () => {
    const first = await startServed({ credential: 'port-stability', port: null })
    const port = first.port
    await first.stop()
    const second = await startServed({
      credential: 'port-stability',
      port: null,
      dataDir: first.dataDir,
      home: first.home
    })
    try {
      expect(second.port).toBe(port)
    } finally {
      await second.stop()
      for (const dir of [first.dataDir, first.home]) {
        fs.rmSync(dir, { recursive: true, force: true })
      }
    }
  }, 60_000)
})
