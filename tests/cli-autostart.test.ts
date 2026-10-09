import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const spawn = vi.hoisted(() => vi.fn(() => ({ unref: vi.fn() })))
vi.mock('node:child_process', () => ({ spawn }))
vi.mock('node:fs', () => ({
  default: { mkdirSync: vi.fn(), openSync: vi.fn(() => 7), closeSync: vi.fn() }
}))
const binary = vi.hoisted(() => ({
  found: { vornd: '/install/vornd', sessiond: '/install/vorn-sessiond' } as {
    vornd: string
    sessiond: string | null
  } | null,
  refused: null as string | null
}))
vi.mock('../packages/server/src/vornd-binary', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../packages/server/src/vornd-binary')>()),
  findVornd: () => binary.found,
  findWebClient: () => null,
  refusesDefault: () => binary.refused
}))

import { ensureServer } from '../packages/server/src/cli/autostart'
import type { RpcTransport } from '../packages/server/src/cli/transport'

/** A transport that is down until the nth question, then up. */
function transportUpAfter(answers: number): RpcTransport {
  let asked = 0
  return { isRunning: () => ++asked > answers } as unknown as RpcTransport
}

const sink = (): { write: (t: string) => void; text: () => string } => {
  const parts: string[] = []
  return { write: (t) => parts.push(t), text: () => parts.join('') }
}

beforeEach(() => {
  spawn.mockClear()
  binary.found = { vornd: '/install/vornd', sessiond: '/install/vorn-sessiond' }
  binary.refused = null
})
afterEach(() => vi.useRealTimers())

describe('starting a server when there is none', () => {
  it('spawns nothing when one is already listening', async () => {
    const err = sink()
    const rpc = { isRunning: () => true } as unknown as RpcTransport

    expect(await ensureServer(rpc, err.write)).toBe(true)
    expect(spawn).not.toHaveBeenCalled()
    expect(err.text()).toBe('')
  })

  it('starts one detached, says so on stderr, and waits for it', async () => {
    const err = sink()

    expect(await ensureServer(transportUpAfter(2), err.write)).toBe(true)
    expect(err.text()).toContain('starting one')

    const [command, args, options] = spawn.mock.calls[0] as unknown as [
      string,
      string[],
      { detached: boolean }
    ]
    expect(command).toBe('/install/vornd')
    expect(args).toContain('--sessiond')
    expect(options.detached).toBe(true)
  })

  it('passes a data directory on, so both halves agree where the server is', async () => {
    const err = sink()
    await ensureServer(transportUpAfter(1), err.write, '/tmp/elsewhere')

    const [, args, options] = spawn.mock.calls[0] as unknown as [string, string[], { cwd: string }]
    expect(args.slice(0, 2)).toEqual(['--data-dir', '/tmp/elsewhere'])
    // The log and the working directory follow the flag, not what discovery found.
    expect(options.cwd).toBe('/tmp/elsewhere')
  })

  it('gives up on a deadline rather than hanging, and says where to look', async () => {
    vi.useFakeTimers()
    const err = sink()
    const rpc = { isRunning: () => false } as unknown as RpcTransport

    const result = ensureServer(rpc, err.write)
    await vi.advanceTimersByTimeAsync(21_000)

    expect(await result).toBe(false)
    expect(err.text()).toContain('server.log')
  })

  it('says so when this install has no vornd', async () => {
    binary.found = null
    const err = sink()

    expect(await ensureServer(transportUpAfter(5), err.write, '/tmp/elsewhere')).toBe(false)
    expect(err.text()).toContain('no vornd')
    expect(spawn).not.toHaveBeenCalled()
  })

  it('starts nothing on the default data directory from a build run from source', async () => {
    binary.refused = 'refused: the default data directory'
    const err = sink()

    expect(await ensureServer(transportUpAfter(5), err.write)).toBe(false)
    expect(err.text()).toContain('refused')
    expect(spawn).not.toHaveBeenCalled()
  })
})
