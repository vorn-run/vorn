import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { leftOver, tiedTo } from './helpers/daemons'

// A stand-in for a session holder: node under the name the check looks for.
function standIn(env: Record<string, string>): { pid: number; exited: Promise<void> } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-daemons-'))
  const fake = path.join(dir, 'vorn-sessiond')
  fs.symlinkSync(process.execPath, fake)
  const child = spawn(fake, ['-e', 'setTimeout(() => {}, 60000)'], {
    env: { ...process.env, ...env },
    stdio: 'ignore'
  })
  return { pid: child.pid!, exited: new Promise((r) => child.once('exit', () => r())) }
}

describe.skipIf(process.platform === 'win32')('the check for daemons a test left behind', () => {
  it('finds the daemons tied to a process, and only those', async () => {
    const mine = standIn({ VORN_EXIT_WITH: String(process.pid) })
    const other = standIn({ VORN_EXIT_WITH: String(process.pid + 1) })
    try {
      await expect
        .poll(() => tiedTo(process.pid).map((d) => d.pid), { timeout: 5000 })
        .toContain(mine.pid)
      expect(tiedTo(process.pid).map((d) => d.pid)).not.toContain(other.pid)

      const said = await leftOver(() => tiedTo(process.pid), 0)
      expect(said).toContain(`${mine.pid} `)
      await mine.exited
      expect(await leftOver(() => tiedTo(process.pid), 0)).toBe('')
    } finally {
      for (const pid of [other.pid, mine.pid]) {
        try {
          process.kill(pid)
        } catch {
          // Already ended.
        }
      }
    }
  }, 30_000)
})
