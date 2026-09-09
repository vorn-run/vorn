import { describe, it, expect, afterEach } from 'vitest'
import path from 'node:path'
import { spawn } from 'node:child_process'
import * as pty from 'node-pty'

/**
 * The claim the whole feature rests on: nothing in a pty names which process owns
 * the master, so a second one can read, write and resize it unnoticed.
 *
 * Two processes on purpose. Node has no `SCM_RIGHTS`, so a descriptor only crosses
 * in a child's `stdio` array -- and in-process, node-pty's own reader is still on it.
 */
const opened: pty.IPty[] = []
afterEach(() => {
  for (const p of opened.splice(0)) {
    try {
      p.kill()
    } catch {
      // Already gone, which is what most of these tests arrange.
    }
  }
})

interface HeirReport {
  read: boolean
  cols: string
  burst: boolean
  exited: boolean
}

async function handToAnotherProcess(): Promise<HeirReport> {
  const shell = pty.spawn('/bin/bash', ['--norc', '--noprofile', '-i'], {
    name: 'xterm-256color',
    cols: 80,
    rows: 24,
    cwd: process.cwd()
  })
  opened.push(shell)

  // Paused the way a handoff pauses it: the bytes wait in the kernel's buffer.
  await new Promise((resolve) => setTimeout(resolve, 400))
  shell.pause()

  const master = (shell as unknown as { fd: number }).fd
  const repoRoot = path.join(__dirname, '..')

  // `--import tsx`, never the binary: the CLI re-executes in a child that inherits
  // nothing. The production dev path had the same bug and this is what found it.
  const heir = spawn(
    process.execPath,
    ['--import', 'tsx', path.join(__dirname, 'fixtures', 'adopt-pty-heir.ts')],
    {
      cwd: repoRoot,
      env: { ...process.env, SHELL_PID: String(shell.pid) },
      // 3 carries the report; the pane follows at 4, which is `FIRST_PTY_SLOT`.
      stdio: ['ignore', 'inherit', 'inherit', 'ipc', master]
    }
  )

  return new Promise<HeirReport>((resolve, reject) => {
    const timer = setTimeout(() => {
      heir.kill('SIGKILL')
      reject(new Error('the adopting process never reported'))
    }, 60_000)
    heir.on('message', (msg) => {
      clearTimeout(timer)
      resolve(msg as HeirReport)
    })
    heir.on('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`the adopting process exited (${code}) without reporting`))
    })
  })
}

describe('a pty handed to another process', () => {
  it('is fully usable by its new owner', async () => {
    const report = await handToAnotherProcess()

    // Reads reach the new owner.
    expect(report.read).toBe(true)

    // The assertion that cannot be faked: Node has no ioctl, so 120 from `tput cols`
    // proves the binding call landed on this descriptor in this process.
    expect(report.cols).toBe('120')

    // Whole rather than truncated at the first EAGAIN.
    expect(report.burst).toBe(true)

    // Visible even though nothing can `waitpid` for a program it never forked.
    expect(report.exited).toBe(true)
  }, 90_000)
})
