import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const spawn = vi.hoisted(() => vi.fn(() => ({ unref: vi.fn() })))
vi.mock('node:child_process', () => ({ spawn }))
vi.mock('node:fs', () => ({
  default: { mkdirSync: vi.fn(), openSync: vi.fn(() => 7), closeSync: vi.fn() }
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

beforeEach(() => spawn.mockClear())
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

    const [, args, options] = spawn.mock.calls[0] as unknown as [
      string,
      string[],
      { detached: boolean }
    ]
    expect(args.slice(-2)).toEqual(['server', 'serve'])
    expect(options.detached).toBe(true)
  })

  it('passes a data directory on, so both halves agree where the server is', async () => {
    const err = sink()
    await ensureServer(transportUpAfter(1), err.write, '/tmp/elsewhere')

    const [, args] = spawn.mock.calls[0] as unknown as [string, string[]]
    expect(args.slice(-2)).toEqual(['--data-dir', '/tmp/elsewhere'])
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
})
