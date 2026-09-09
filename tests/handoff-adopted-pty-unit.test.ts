import { describe, it, expect, afterEach } from 'vitest'
import fs from 'node:fs'
import { spawn, type ChildProcess } from 'node:child_process'
import * as pty from 'node-pty'
import { AdoptedPty } from '../packages/server/src/handoff/adopted-pty'

/**
 * The adopted pty against a bare master/slave pair.
 *
 * `native.open` hands back two descriptors with no node-pty stream attached to
 * either, which is the only way to exercise this in one process: a real terminal
 * to read and write, and nothing else competing for its bytes. The cross-process
 * test beside this one proves the same class works when the descriptor was
 * inherited rather than opened here.
 */
/**
 * A pid nothing owns, so `kill()` takes the ESRCH path instead of signalling.
 *
 * The obvious choice, this process's own pid, would have `kill()` send SIGHUP to
 * the test runner. Only the test that is actually about signalling uses a real one.
 */
const NOBODY = 2147483646

const live: AdoptedPty[] = []
const slaves: number[] = []
const started: ChildProcess[] = []

afterEach(() => {
  // Through `kill`, never `fs.closeSync(master)`: the reader owns that descriptor,
  // and closing it underneath libuv corrupts whatever reuses the number next --
  // which is the following test in this file.
  for (const adopted of live.splice(0)) adopted.kill()
  for (const fd of slaves.splice(0)) {
    try {
      fs.closeSync(fd)
    } catch {
      // Already closed by the test that was checking what happens when it is.
    }
  }
  for (const child of started.splice(0)) {
    try {
      child.kill('SIGKILL')
    } catch {
      // Already gone.
    }
  }
})

function terminal(): { master: number; slave: number } {
  const pair = (
    pty as unknown as {
      native: { open(cols: number, rows: number): { master: number; slave: number } }
    }
  ).native.open(80, 24)
  slaves.push(pair.slave)
  return pair
}

function adopt(master: number, pid: number = NOBODY): AdoptedPty {
  const adopted = new AdoptedPty(master, pid)
  live.push(adopted)
  return adopted
}

const settled = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 150))

describe('a pty adopted in this process', () => {
  it('delivers what the far side writes', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    const seen: string[] = []
    adopted.onData((d) => seen.push(d))

    fs.writeSync(slave, 'from the program\n')
    await settled()
    expect(seen.join('')).toContain('from the program')
  })

  it('holds output that arrived before anything was listening', async () => {
    // Attaching the reader starts the flow, and a handoff pauses a tick later.
    // Anything read in that window has already left the kernel's buffer, so
    // dropping it would lose output nothing can get back.
    const { master, slave } = terminal()
    const adopted = adopt(master)
    fs.writeSync(slave, 'said before anyone listened\n')
    await settled()

    const seen: string[] = []
    adopted.onData((d) => seen.push(d))
    expect(seen.join('')).toContain('said before anyone listened')
  })

  it('writes through to the far side', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    adopted.write('typed at the terminal\n')
    await settled()

    const buffer = Buffer.alloc(256)
    const read = fs.readSync(slave, buffer, 0, buffer.length, null)
    expect(buffer.subarray(0, read).toString()).toContain('typed at the terminal')
  })

  it('writes a burst larger than the kernel buffer without truncating it', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    // Newline-delimited: master-to-slave is the program's *input*, and a canonical
    // line discipline holds an unterminated line rather than passing it on.
    const payload = `${'x'.repeat(200)}\n`.repeat(320)

    let received = 0
    const drained = new Promise<void>((resolve, reject) => {
      const buffer = Buffer.alloc(16 * 1024)
      const pump = (): void => {
        fs.read(slave, buffer, 0, buffer.length, null, (err, bytes) => {
          // The descriptor is non-blocking, so "nothing yet" arrives as an error.
          if (err) {
            if ((err as NodeJS.ErrnoException).code !== 'EAGAIN') return reject(err)
            return setImmediate(pump)
          }
          received += bytes
          if (received >= payload.length) return resolve()
          pump()
        })
      }
      pump()
    })

    // One call, far past the pty buffer. Without the retry queue this is where a
    // paste is silently truncated at the first EAGAIN.
    adopted.write(payload)
    await drained
    expect(received).toBeGreaterThanOrEqual(payload.length)
  }, 20_000)

  it('stops and starts reading without losing anything', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    const seen: string[] = []
    adopted.onData((d) => seen.push(d))

    adopted.pause()
    fs.writeSync(slave, 'written while paused\n')
    await settled()
    expect(seen.join('')).not.toContain('written while paused')

    adopted.resume()
    await settled()
    expect(seen.join('')).toContain('written while paused')
  })

  it('resizes without throwing', () => {
    const { master } = terminal()
    const adopted = adopt(master)
    // The ioctl reaching the real tty is what the cross-process test proves with
    // `tput cols`; there is no portable way to read a winsize back here.
    expect(() => adopted.resize(120, 40)).not.toThrow()
  })

  it('survives the read errors a non-blocking descriptor raises normally', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    let ended = false
    adopted.onExit(() => {
      ended = true
    })
    const seen: string[] = []
    adopted.onData((d) => seen.push(d))

    // node-pty's own reader notes this arrives twice on startup. Reading it as
    // the far side going away would end a healthy terminal the instant it was
    // adopted -- and `onExit` takes the session's history with it.
    const transient: NodeJS.ErrnoException = new Error('resource temporarily unavailable')
    transient.code = 'EAGAIN'
    ;(adopted as unknown as { reader: { emit(event: string, err: Error): void } }).reader.emit(
      'error',
      transient
    )

    expect(ended).toBe(false)
    fs.writeSync(slave, 'still here\n')
    await settled()
    expect(seen.join('')).toContain('still here')
  })

  it('reports the far side closing', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    const ended = new Promise<{ exitCode: number }>((resolve) => adopted.onExit(resolve))

    fs.closeSync(slave)
    // The exit code is genuinely unavailable to an adopter, so zero stands in.
    expect((await ended).exitCode).toBe(0)
  })

  it('signals the process behind it', async () => {
    const { master } = terminal()
    const child = spawn('sleep', ['30'])
    started.push(child)
    const adopted = adopt(master, child.pid as number)

    const gone = new Promise<void>((resolve) => child.once('exit', () => resolve()))
    adopted.kill('SIGKILL')
    await gone
    expect(child.killed || child.exitCode !== null || child.signalCode !== null).toBe(true)
  })

  it('treats a process that has already gone as ended, not an error', () => {
    const { master } = terminal()
    // A pid nothing owns: ESRCH is the ordinary case, not something to log about.
    const adopted = adopt(master)
    let ended = false
    adopted.onExit(() => {
      ended = true
    })
    expect(() => adopted.kill()).not.toThrow()
    expect(ended).toBe(true)
  })

  it('goes quiet once it has ended', async () => {
    const { master } = terminal()
    const adopted = adopt(master)
    adopted.kill()

    // Nothing is written to the slave here: ending destroys the reader, which
    // closes the master, so the far side would raise EIO -- the test would be
    // asserting on the pty rather than on this class.
    const seen: string[] = []
    adopted.onData((d) => seen.push(d))
    adopted.write('nobody should see this')
    adopted.resize(90, 30)
    await settled()
    expect(seen).toEqual([])
  })

  it('lets a listener stop listening', async () => {
    const { master, slave } = terminal()
    const adopted = adopt(master)
    const seen: string[] = []
    const subscription = adopted.onData((d) => seen.push(d))

    subscription.dispose()
    fs.writeSync(slave, 'after the listener left\n')
    await settled()
    expect(seen).toEqual([])
  })
})
