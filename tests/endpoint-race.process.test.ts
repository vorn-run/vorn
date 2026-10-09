import { describe, it, expect, afterEach } from 'vitest'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import WebSocket from 'ws'
import { EXIT_ENDPOINT_TAKEN } from '../packages/shared/src/protocol'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { builtVornd, startServed, type Served } from './helpers/served'

spawnsRealServers()

/**
 * Two real vornds, one data directory: the second stands down, the first keeps
 * serving its clients, and a directory whose owner is gone is free again.
 * Each run has its own home and data directory.
 */

const CREDENTIAL = 'endpoint-race-credential'
const started: Served[] = []
const dirs: string[] = []

afterEach(async () => {
  for (const s of started.splice(0)) await s.stop()
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true })
})

async function first(): Promise<Served> {
  const served = await startServed({ credential: CREDENTIAL })
  started.push(served)
  dirs.push(served.dataDir, served.home)
  return served
}

/** A second vornd on `served`'s directory, run to its exit. */
function second(served: Served): Promise<number | null> {
  const child = spawn(builtVornd!, ['--data-dir', served.dataDir, '--port', '0'], {
    stdio: 'ignore',
    env: { ...process.env, HOME: served.home, USERPROFILE: served.home, VORND_KEYCHAIN: '0' }
  })
  return new Promise((resolve) => child.once('exit', (code) => resolve(code)))
}

const published = (served: Served): { port: number; pid: number } =>
  JSON.parse(fs.readFileSync(path.join(served.dataDir, 'ws-port'), 'utf-8'))

describe.skipIf(!builtVornd || process.platform === 'win32')(
  'two vornds for one data directory',
  () => {
    it('leaves the directory with the first, and the second stands down', async () => {
      const one = await first()
      expect(await second(one)).toBe(EXIT_ENDPOINT_TAKEN)
      expect(published(one)).toEqual({ port: one.port, pid: one.child.pid })
    }, 60_000)

    it('never drops a client held open across the attempt', async () => {
      const one = await first()
      const ws = new WebSocket(`ws://127.0.0.1:${one.port}/ws`, {
        headers: { authorization: `Bearer ${CREDENTIAL}` }
      })
      await new Promise((resolve, reject) => {
        ws.once('open', resolve)
        ws.once('error', reject)
      })
      let closed = false
      ws.once('close', () => (closed = true))
      expect(await second(one)).toBe(EXIT_ENDPOINT_TAKEN)
      const answered = new Promise<unknown>((resolve) =>
        ws.on('message', (raw) => {
          const frame = JSON.parse(String(raw)) as { id?: number; result?: unknown }
          if (frame.id === 1) resolve(frame.result)
        })
      )
      ws.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'config:load' }))
      expect(await answered).toHaveProperty('projects')
      expect(closed).toBe(false)
      ws.close()
    }, 60_000)

    it('lets the next start have a directory its owner left, cleanly or killed', async () => {
      const one = await first()
      await one.stop()
      const two = await startServed({
        credential: CREDENTIAL,
        dataDir: one.dataDir,
        home: one.home
      })
      started.push(two)
      two.child.kill('SIGKILL')
      await new Promise((resolve) => two.child.once('exit', resolve))
      const three = await startServed({
        credential: CREDENTIAL,
        dataDir: one.dataDir,
        home: one.home
      })
      started.push(three)
      expect(published(three).pid).toBe(three.child.pid)
    }, 60_000)
  }
)
