import { describe, it, expect } from 'vitest'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'

/** vornd stopping cleanly once the app that read its stderr has gone. */
const vornd = [
  path.resolve(__dirname, '../packages/core/vornd'),
  path.resolve(__dirname, '../packages/core/target/release/vornd')
].find((p): p is string => !!p && fs.existsSync(p))

async function stopWith(
  stderr: 'read' | 'gone'
): Promise<{ code: number | null; signal: string | null }> {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-stop-'))
  try {
    const child = spawn(
      vornd!,
      ['--data-dir', path.join(home, 'data'), '--port', '0', '--exit-with-stdin'],
      {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, HOME: home, VORND_KEYCHAIN: '0', VORND_LOG: 'info' }
      }
    )
    await new Promise((resolve) => createInterface({ input: child.stdout! }).once('line', resolve))
    if (stderr === 'gone') child.stderr!.destroy()
    else child.stderr!.resume()
    const exited = new Promise<{ code: number | null; signal: string | null }>((resolve) =>
      child.once('exit', (code, signal) => resolve({ code, signal }))
    )
    // As the app exits: its end of the stderr pipe goes before vornd hears stdin close.
    await new Promise((r) => setTimeout(r, 200))
    child.stdin!.end()
    return await exited
  } finally {
    fs.rmSync(home, { recursive: true, force: true })
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
