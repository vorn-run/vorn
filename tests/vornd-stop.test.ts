import { describe, it, expect } from 'vitest'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { createInterface } from 'node:readline'

/**
 * vornd stopping after the server that started it has gone.
 *
 * The server reads vornd's stderr and exits first, so vornd's last log lines
 * go to a pipe nobody reads. A failed write used to be reported with a print
 * that panics on a broken stderr, so vornd ended with exit code 101 before it
 * wrote its session records down or took its last checkpoints.
 *
 * Runs where vornd has been built (`yarn build:core`, or `VORN_CONFORMANCE_VORND`).
 */
const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, '../packages/core/vornd'),
  path.resolve(__dirname, '../packages/core/target/release/vornd')
].find((p): p is string => !!p && fs.existsSync(p))

async function stopWith(
  stderr: 'read' | 'gone'
): Promise<{ code: number | null; signal: string | null }> {
  const upstream = http.createServer((_, res) => res.end('ok'))
  await new Promise<void>((resolve) => upstream.listen(0, '127.0.0.1', () => resolve()))
  const address = upstream.address()
  const port = typeof address === 'object' && address ? address.port : 0
  try {
    const child = spawn(vornd!, ['--upstream', `127.0.0.1:${port}`, '--exit-with-stdin'], {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, VORND_LOG: 'info' }
    })
    await new Promise((resolve) => createInterface({ input: child.stdout! }).once('line', resolve))
    if (stderr === 'gone') child.stderr!.destroy()
    else child.stderr!.resume()
    const exited = new Promise<{ code: number | null; signal: string | null }>((resolve) =>
      child.once('exit', (code, signal) => resolve({ code, signal }))
    )
    // As the server exits: its end of the stderr pipe goes before vornd hears stdin close.
    await new Promise((r) => setTimeout(r, 200))
    child.stdin!.end()
    return await exited
  } finally {
    upstream.close()
  }
}

describe.skipIf(!vornd || process.platform === 'win32')('vornd stopping', () => {
  it('stops cleanly when its stdin closes, its stderr read', async () => {
    expect(await stopWith('read')).toEqual({ code: 0, signal: null })
  })

  it('stops cleanly when nothing reads its stderr any more', async () => {
    expect(await stopWith('gone')).toEqual({ code: 0, signal: null })
  })
})
