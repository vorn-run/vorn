import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

// The same isolation as hook-payload.test.ts: a real HookServer writes to a home.
vi.mock('node-pty', () => ({ default: { spawn: vi.fn() }, spawn: vi.fn() }))

const holderAlive = { value: true }
vi.mock('../packages/server/src/hook-ownership', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../packages/server/src/hook-ownership')>()),
  pidIsAlive: (pid: number) => (pid === 4242 ? holderAlive.value : pid === process.pid)
}))

let home: string | null = null
let realHome: string | undefined
let realProfile: string | undefined

beforeAll(() => {
  realHome = process.env.HOME
  realProfile = process.env.USERPROFILE
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-hook-owner-'))
  process.env.HOME = home
  process.env.USERPROFILE = home
  fs.mkdirSync(path.join(home, '.vorn'))
})

function restore(name: 'HOME' | 'USERPROFILE', value: string | undefined): void {
  if (value === undefined) delete process.env[name]
  else process.env[name] = value
}

afterAll(() => {
  restore('HOME', realHome)
  restore('USERPROFILE', realProfile)
  if (home) fs.rmSync(home, { recursive: true, force: true })
})

const started: Array<{ stop: () => void }> = []
afterEach(() => {
  for (const s of started.splice(0)) s.stop()
  holderAlive.value = true
})

const ownerFile = (): string => path.join(home!, '.vorn', 'hook-owner')

describe('a server that found the registration held', () => {
  it('claims it once the holder is gone, and says so', async () => {
    fs.writeFileSync(ownerFile(), JSON.stringify({ port: 1, pid: 4242 }))
    const { HookServer } = await import('../packages/server/src/hook-server')
    HookServer.OWNER_POLL_MS = 20
    const server = new HookServer()
    started.push(server)
    const claimed = new Promise<number>((resolve) => server.once('claimed', resolve))

    const port = await server.start(0)
    expect(server.ownsRegistration()).toBe(false)
    expect(JSON.parse(fs.readFileSync(ownerFile(), 'utf-8')).pid).toBe(4242)

    holderAlive.value = false
    expect(await claimed).toBe(port)
    expect(server.ownsRegistration()).toBe(true)
    expect(JSON.parse(fs.readFileSync(ownerFile(), 'utf-8'))).toEqual({ port, pid: process.pid })
    expect(fs.readFileSync(path.join(home!, '.vorn', 'port'), 'utf-8')).toBe(String(port))
    expect(fs.readFileSync(path.join(home!, '.vorn', 'token'), 'utf-8')).toBe(server.getAuthToken())
  })

  it('stops looking when it is stopped first', async () => {
    fs.writeFileSync(ownerFile(), JSON.stringify({ port: 1, pid: 4242 }))
    const { HookServer } = await import('../packages/server/src/hook-server')
    HookServer.OWNER_POLL_MS = 20
    const server = new HookServer()
    const onClaimed = vi.fn()
    server.on('claimed', onClaimed)
    await server.start(0)
    server.stop()
    holderAlive.value = false
    await new Promise((r) => setTimeout(r, 80))
    expect(onClaimed).not.toHaveBeenCalled()
    expect(JSON.parse(fs.readFileSync(ownerFile(), 'utf-8')).pid).toBe(4242)
  })
})
