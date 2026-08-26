import { describe, it, expect, afterEach } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/**
 * Does the process actually leave?
 *
 * The pure tests pin the decision; this pins the only thing they cannot — that a
 * real server, told nothing is happening, exits. It is the claim the feature is
 * named for, and it was untested until a review pointed out that every green
 * test was asserting a snapshot rather than a process.
 *
 * HOME is redirected along with --data-dir, and that is not tidiness: `shutdown()`
 * calls `uninstallHooks()`, which writes `~/.claude/settings.json` regardless of
 * the data dir. A test that skipped this would uninstall the developer's own
 * agent hooks on the way past.
 */

const ENTRY = path.join(__dirname, '..', 'packages', 'server', 'dist', 'index.cjs')
const built = fs.existsSync(ENTRY)

let child: ChildProcess | null = null
let dir: string | null = null

function launch(idleMs: number): ChildProcess {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-idle-'))
  fs.mkdirSync(path.join(dir, '.vorn'), { recursive: true })
  const proc = spawn(
    process.execPath,
    [ENTRY, '--data-dir', path.join(dir, '.vorn'), '--port', '0'],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
      env: {
        ...process.env,
        HOME: dir,
        USERPROFILE: dir,
        VORN_IDLE_TIMEOUT_MS: String(idleMs)
      }
    }
  )
  child = proc
  return proc
}

const alive = (pid: number): boolean => {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

afterEach(() => {
  if (child?.pid && alive(child.pid)) {
    try {
      process.kill(-child.pid, 'SIGKILL')
    } catch {
      try {
        process.kill(child.pid, 'SIGKILL')
      } catch {
        /* already gone */
      }
    }
  }
  child = null
  if (dir) fs.rmSync(dir, { recursive: true, force: true })
  dir = null
})

describe.skipIf(!built)('a server with nothing to do', () => {
  it('exits on its own', async () => {
    const proc = launch(2_000)
    const pid = proc.pid as number
    await new Promise((r) => setTimeout(r, 12_000))
    expect(alive(pid)).toBe(false)
  }, 40_000)

  it('stays up while a websocket client keeps sending frames', async () => {
    // The counterpart to the first test, and the one that proves the window
    // measures traffic rather than elapsed time. These sockets have no
    // heartbeat, so only real frames reset the clock.
    const proc = launch(2_000)
    const pid = proc.pid as number
    const portFile = path.join(dir as string, '.vorn', 'ws-port')
    const tokenFile = path.join(dir as string, '.vorn', 'local-token')
    for (let i = 0; i < 60 && !(fs.existsSync(portFile) && fs.existsSync(tokenFile)); i++) {
      await new Promise((r) => setTimeout(r, 250))
    }
    expect(fs.existsSync(portFile)).toBe(true)

    const { port } = JSON.parse(fs.readFileSync(portFile, 'utf-8'))
    const token = fs.readFileSync(tokenFile, 'utf-8').trim()
    const { default: WebSocket } = await import('ws')
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { Authorization: `Bearer ${token}` }
    })
    await new Promise((r) => ws.once('open', r))

    // Well past the window, sending the whole time.
    const deadline = Date.now() + 9_000
    let n = 0
    while (Date.now() < deadline) {
      ws.send(JSON.stringify({ jsonrpc: '2.0', id: ++n, method: 'config:load' }))
      await new Promise((r) => setTimeout(r, 500))
    }
    const stillUp = alive(pid)
    ws.close()

    expect(stillUp).toBe(true)
  }, 45_000)
})
