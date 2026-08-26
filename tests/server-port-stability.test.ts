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
// The tsx binary itself, not `npx tsx`. `npx` is a parent of the process that
// actually listens on some platforms and execs into it on others, and that
// difference is the whole reason this test passed locally and failed in CI.
// Spawning the binary removes the layer rather than reasoning about it.
const TSX = path.join(__dirname, '..', 'node_modules', '.bin', 'tsx')

let child: ChildProcess | null = null
let sandbox: string | null = null

afterEach(() => {
  if (child) killGroup(child, 'SIGKILL')
  child = null
  if (sandbox) fs.rmSync(sandbox, { recursive: true, force: true })
  sandbox = null
})

/** Boot the server the way Electron does, and read the port it announces. */
function launch(dir: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const proc = spawn(TSX, [ENTRY, '--data-dir', path.join(dir, 'data')], {
      stdio: 'pipe',
      // Nothing this server writes may escape the sandbox. `os.homedir()` follows
      // HOME, which is what keeps `~/.claude/settings.json` and `~/.vorn` out of
      // reach — see the note above.
      env: { ...process.env, HOME: dir, USERPROFILE: dir },
      // Its own process group, so it can be killed as one. `npx tsx` is a parent
      // of the process that actually listens, and signalling only the parent
      // leaves the child holding the port — which is precisely how the second
      // launch below ends up on a fallback port and this test fails for a reason
      // that has nothing to do with what it is checking.
      detached: true
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

/** Signal the whole group; the listener is a child of the process we spawned. */
function killGroup(proc: ChildProcess, signal: NodeJS.Signals): void {
  if (proc.pid === undefined) return
  try {
    process.kill(-proc.pid, signal)
  } catch {
    // Already gone, or never became a group leader.
  }
}

/** Whether any process in the group is still alive. */
function groupAlive(pid: number): boolean {
  try {
    process.kill(-pid, 0) // a probe, not a signal
    return true
  } catch {
    return false
  }
}

/**
 * Down, and confirmed down.
 *
 * Waiting on the spawned process is not enough on its own: a server still holding
 * its port makes the relaunch bind a fallback, failing the assertion for a reason
 * that has nothing to do with what is being checked. That is what happened in CI,
 * and it passed locally only because this machine already had something on the
 * default port, so both launches overlapped on it through `SO_REUSEADDR` and
 * neither ever reached `EADDRINUSE`. Hence the process group, and the wait.
 *
 * The wait is on the process group rather than on the port being free. A port can
 * be held by something that is not ours — locally, the default is exactly that —
 * so "nothing answers there" is not a precondition this test can ever demand.
 */
async function stop(): Promise<void> {
  const proc = child
  child = null
  if (!proc) return

  const pid = proc.pid
  await new Promise<void>((resolve) => {
    let settled = false
    const done = () => {
      if (settled) return
      settled = true
      resolve()
    }
    proc.once('exit', done)
    killGroup(proc, 'SIGTERM')
    setTimeout(done, 10_000)
  })

  if (pid === undefined) return
  for (let i = 0; i < 100; i++) {
    if (!groupAlive(pid)) return
    if (i === 50) killGroup(proc, 'SIGKILL')
    await new Promise((r) => setTimeout(r, 100))
  }
  throw new Error('the server process group was still alive 10s after being killed')
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
