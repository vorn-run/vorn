import { describe, it, expect, afterEach } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/**
 * The one assertion the unit tests cannot make: two launches agree.
 *
 * `resolveServerPort` and `shouldRememberPort` are pure and thoroughly covered,
 * and they would all stay green against the bug this file exists for. That bug
 * was not in the decision but in the wiring to it — the direct-run entry point
 * passed `port ?? 0`, turning "no flag given" into an explicit zero before the
 * decision ever saw it, so the remembered port was written every launch and read
 * on none. Checked, not assumed: restoring the `?? 0` gives two launches 65089
 * and 65099, with every unit test still passing.
 *
 * `tests/port-file-lifecycle.test.ts` takes the other road — it says the server
 * is "expensive to spin up in tests" and replicates the logic against a temp
 * directory instead. That is why this one boots the real entry point. A test
 * that reimplements what it is checking cannot fail when the original changes.
 *
 * The invariant holds whatever the machine is doing, which is what makes it safe
 * in CI. Either the first launch bound the port it asked for and remembered it,
 * or the default was busy and it remembered the fallback instead — a fresh data
 * directory has nothing to protect. Both leave the second launch bound to the
 * same port.
 *
 * `HOME` is redirected as well as the data directory, and that is not tidiness.
 * `hook-installer` writes Vorn's hook entry into `~/.claude/settings.json` and the
 * hook server writes `~/.vorn/port` and `~/.vorn/token`, none of which look at
 * `--data-dir` — they resolve from `os.homedir()`. A test server left to its own
 * devices registers itself as the machine's hook endpoint, over whatever Vorn the
 * person is actually running, and a run killed mid-flight leaves that pointing at
 * a port with nothing behind it. Every path this process writes has to land inside
 * the temp directory.
 */

const ENTRY = path.join(__dirname, '..', 'packages', 'server', 'src', 'index.ts')

let child: ChildProcess | null = null
let sandbox: string | null = null

afterEach(() => {
  child?.kill('SIGTERM')
  child = null
  if (sandbox) fs.rmSync(sandbox, { recursive: true, force: true })
  sandbox = null
})

/** Boot the server the way Electron does, and read the port it announces. */
function launch(dir: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const proc = spawn('npx', ['tsx', ENTRY, '--data-dir', path.join(dir, 'data')], {
      stdio: 'pipe',
      // Nothing this server writes may escape the sandbox. `os.homedir()` follows
      // HOME, which is what keeps `~/.claude/settings.json` and `~/.vorn` out of
      // reach — see the note above.
      env: { ...process.env, HOME: dir, USERPROFILE: dir }
    })
    child = proc
    let out = ''
    const timer = setTimeout(
      () => reject(new Error(`no port announced; stdout was: ${out}`)),
      60_000
    )

    proc.stdout?.on('data', (chunk: Buffer) => {
      out += chunk.toString()
      // The `{"port":N}` line is the contract with the launcher, written by the
      // direct-run block — the very block this test is here to guard.
      const found = out.match(/"port":\s*(\d+)/)
      if (!found) return
      clearTimeout(timer)
      resolve(Number(found[1]))
    })
    proc.on('error', (err) => {
      clearTimeout(timer)
      reject(err)
    })
  })
}

/** Down, and confirmed down, so the next launch is not racing a held socket. */
async function stop(): Promise<void> {
  const proc = child
  child = null
  if (!proc) return
  await new Promise<void>((resolve) => {
    proc.once('exit', () => resolve())
    proc.kill('SIGTERM')
    setTimeout(() => {
      proc.kill('SIGKILL')
      resolve()
    }, 10_000)
  })
}

describe('a server relaunched on the same data directory', () => {
  it('comes back on the port it used before', async () => {
    sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-port-'))

    const first = await launch(sandbox)
    await stop()
    const second = await launch(sandbox)

    // Not merely "a port" — the same one. A device paired to the first is still
    // paired after the second, which is the whole reason any of this exists.
    expect(second).toBe(first)
    expect(first).toBeGreaterThan(0)
  }, 180_000)
})
