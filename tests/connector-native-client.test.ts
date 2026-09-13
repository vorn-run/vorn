import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
// The client's own composition is under test, not the login shell the real base is read from.
vi.mock('../packages/server/src/process-utils', () => ({
  getSafeEnv: () => ({
    PATH: process.env.PATH ?? '',
    SAFE_BASE: 'base',
    SHARED: 'base'
  })
}))

import log from '../packages/server/src/logger'
import {
  SdkCallError,
  SdkTransportError,
  createFrameReader,
  errorLine,
  startNativeClient,
  type ChildExit,
  type NativeClient,
  type NativeClientOptions
} from '../packages/server/src/connectors/native-client'

const FIXTURE = path.join(__dirname, 'fixtures', 'native-connector.mjs')
const HELLO_PARAMS = { protocols: [1], host: { name: 'vorn', version: 'test' } }

const started: NativeClient[] = []

async function start(
  mode: string,
  options: Partial<NativeClientOptions> = {},
  env: Record<string, string> = {}
): Promise<NativeClient> {
  const client = await startNativeClient(
    { command: process.execPath, args: [FIXTURE, mode], env },
    { label: 'test', key: mode, ...options }
  )
  started.push(client)
  return client
}

function exitOf(client: NativeClient): Promise<ChildExit> {
  return new Promise((resolve) => client.onExit(resolve))
}

beforeEach(() => {
  vi.mocked(log.info).mockClear()
  vi.mocked(log.warn).mockClear()
})

afterEach(async () => {
  await Promise.all(started.splice(0).map((client) => client.close()))
})

describe('reading frames from a byte stream', () => {
  function reader(maxBytes = 64): {
    lines: string[]
    oversized: number
    feed(chunk: string | Buffer): void
  } {
    const state = { lines: [] as string[], oversized: 0 }
    const feed = createFrameReader({
      maxBytes,
      onLine: (line) => state.lines.push(line),
      onOversize: () => {
        state.oversized += 1
      }
    })
    return {
      get lines() {
        return state.lines
      },
      get oversized() {
        return state.oversized
      },
      feed: (chunk) => feed(typeof chunk === 'string' ? Buffer.from(chunk, 'utf8') : chunk)
    }
  }

  it('joins a line written across chunks and splits several in one', () => {
    const r = reader()
    r.feed('{"a":')
    r.feed('1}\n{"b":2}\n{"c"')
    r.feed(':3}\n')
    expect(r.lines).toEqual(['{"a":1}', '{"b":2}', '{"c":3}'])
  })

  it('keeps a character whose bytes arrive in different chunks', () => {
    const r = reader()
    const bytes = Buffer.from('"✓ é"\n', 'utf8')
    for (let at = 0; at < bytes.length; at += 1) r.feed(bytes.subarray(at, at + 1))
    expect(r.lines).toEqual(['"✓ é"'])
  })

  it('drops a carriage return and blank lines', () => {
    const r = reader()
    r.feed('one\r\n\n  \r\ntwo\n')
    expect(r.lines).toEqual(['one', 'two'])
  })

  it('gives up once a line outgrows the limit, with or without its newline', () => {
    const whole = reader(8)
    whole.feed('0123456789\nlater\n')
    expect(whole.oversized).toBe(1)
    expect(whole.lines).toEqual([])

    const partial = reader(8)
    partial.feed('01234')
    partial.feed('56789')
    partial.feed('\nlater\n')
    expect(partial.oversized).toBe(1)
    expect(partial.lines).toEqual([])
  })

  it('names the last line that reads as an error', () => {
    expect(errorLine(['starting', 'TypeError: nope', 'at x'])).toBe('TypeError: nope')
    expect(errorLine(['starting', 'stopped', ''])).toBe('stopped')
    expect(errorLine([])).toBeUndefined()
  })
})

describe('a native connector child', () => {
  it('answers each request it is sent', async () => {
    const client = await start('normal')
    expect(client.pid).toEqual(expect.any(Number))
    expect(await client.request('vorn/hello', HELLO_PARAMS, 5_000)).toMatchObject({
      protocol: 1,
      connector: { id: 'native-fixture' }
    })
    expect(await client.request('connector/preflight', {}, 5_000)).toEqual({
      ok: true,
      message: 'ready'
    })
  })

  it('reads a reply dribbled a byte at a time, with CRLF and blank lines', async () => {
    const client = await start('split')
    expect(await client.request('connector/manifest', {}, 10_000)).toMatchObject({
      name: 'Native fixture ✓ é'
    })
  })

  it('skips lines that are not replies and still hears the real one', async () => {
    const client = await start('garbage')
    expect(await client.request('connector/preflight', {}, 5_000)).toEqual({
      ok: true,
      message: 'ready'
    })
    expect(vi.mocked(log.warn).mock.calls.map(([line]) => line)).toEqual(
      expect.arrayContaining([
        '[test] garbage wrote a line that is not JSON: not json',
        '[test] garbage wrote a line that is not a message: [1,2]',
        '[test] garbage answered a request nobody is waiting for: 999',
        '[test] garbage sent log, which a connector does not send in protocol 1'
      ])
    )
  })

  it('matches replies that come back out of order to the requests that asked', async () => {
    const client = await start('normal')
    const finished: string[] = []
    const slow = client
      .request('action/run', { action: 'a', args: { name: 'slow', delayMs: 200 } }, 5_000)
      .then((result) => {
        finished.push('slow')
        return result
      })
    const fast = client
      .request('action/run', { action: 'a', args: { name: 'fast', delayMs: 10 } }, 5_000)
      .then((result) => {
        finished.push('fast')
        return result
      })
    expect(await slow).toMatchObject({ echo: { name: 'slow' } })
    expect(await fast).toMatchObject({ echo: { name: 'fast' } })
    expect(finished).toEqual(['fast', 'slow'])
  })

  it('times a request out without stopping the child, and ignores its late reply', async () => {
    const client = await start('slow')
    const late = client.request('action/run', { action: 'a', args: {} }, 50)
    await expect(late).rejects.toBeInstanceOf(SdkTransportError)
    await expect(late).rejects.toMatchObject({ reason: 'timeout' })
    expect(client.exited).toBe(false)
    expect(
      await client.request('action/run', { action: 'a', args: { again: true } }, 5_000)
    ).toMatchObject({ echo: { again: true } })
    await vi.waitFor(() =>
      expect(log.warn).toHaveBeenCalledWith(
        '[test] slow answered a request nobody is waiting for: 1'
      )
    )
  })

  it('carries the error a connector answers with, and its kind', async () => {
    const client = await start('normal')
    const failed = client.request('action/run', { action: 'a', args: { fail: true } }, 5_000)
    await expect(failed).rejects.toBeInstanceOf(SdkCallError)
    await expect(failed).rejects.toMatchObject({
      method: 'action/run',
      code: -32000,
      message: 'the upstream said no',
      kind: 'upstream',
      retryable: true,
      field: 'title'
    })

    const helloError = await start('hello-error')
    await expect(helloError.request('vorn/hello', HELLO_PARAMS, 5_000)).rejects.toMatchObject({
      code: -32000,
      kind: 'internal',
      retryable: false
    })
    const unsupported = await start('hello-unsupported')
    await expect(unsupported.request('vorn/hello', HELLO_PARAMS, 5_000)).rejects.toMatchObject({
      code: -32001,
      message: 'this connector speaks protocol 2'
    })
  })

  it('times out a child that never answers', async () => {
    const client = await start('silent')
    await expect(client.request('vorn/hello', HELLO_PARAMS, 100)).rejects.toMatchObject({
      reason: 'timeout',
      message: '[test] silent did not answer vorn/hello within 0 s'
    })
  })

  it('stops a child whose line outgrows the frame limit', async () => {
    const client = await start('oversized', { maxFrameBytes: 4096 })
    const exit = exitOf(client)
    await client.request('vorn/hello', HELLO_PARAMS, 5_000)
    await expect(client.request('connector/manifest', {}, 10_000)).rejects.toMatchObject({
      reason: 'oversized'
    })
    expect(await exit).toEqual({ code: null, signal: 'SIGKILL' })
  })

  it('names the error a child printed when it exits before answering', async () => {
    const client = await start('crash-on-start')
    const failed = client.request('vorn/hello', HELLO_PARAMS, 5_000)
    await expect(failed).rejects.toMatchObject({ reason: 'exited', exitCode: 3 })
    await expect(failed).rejects.toThrow('Error: the fixture could not start')
    expect(client.stderrTail()).toEqual(['starting', 'Error: the fixture could not start'])
    expect(client.exited).toBe(true)
  })

  it('fails the call a child exits in the middle of, with its code', async () => {
    const client = await start('exit-mid-call')
    await client.request('vorn/hello', HELLO_PARAMS, 5_000)
    const failed = client.request('action/run', { action: 'a', args: {} }, 5_000)
    await expect(failed).rejects.toMatchObject({ reason: 'exited', exitCode: 7 })
    await expect(failed).rejects.toThrow('Error: lost the upstream')
    await expect(client.request('connector/manifest', {}, 5_000)).rejects.toMatchObject({
      reason: 'exited'
    })
  })

  it('keeps only the last lines of stderr, and logs every one', async () => {
    const client = await start('stderr-flood', { stderrTailLines: 5 })
    await client.request('vorn/hello', HELLO_PARAMS, 5_000)
    await vi.waitFor(() => expect(client.stderrTail().at(-1)).toBe('line 100'))
    expect(client.stderrTail()).toEqual(['line 96', 'line 97', 'line 98', 'line 99', 'line 100'])
    expect(log.info).toHaveBeenCalledWith('[test] stderr-flood stderr: line 1')
  })

  it('gives the child the safe base plus its own env, and nothing else from the server', async () => {
    process.env.VORN_FIXTURE_LEAK = 'leaked'
    try {
      const client = await start('normal', {}, { SHARED: 'mine', OWN: 'own' })
      const read = async (name: string): Promise<unknown> =>
        (await client.request('action/run', { action: 'env', args: { env: name } }, 5_000)).value
      expect(await read('SAFE_BASE')).toBe('base')
      expect(await read('SHARED')).toBe('mine')
      expect(await read('OWN')).toBe('own')
      expect(await read('VORN_FIXTURE_LEAK')).toBeNull()
    } finally {
      delete process.env.VORN_FIXTURE_LEAK
    }
  })

  it('refuses to start a command that does not exist', async () => {
    const failed = startNativeClient(
      { command: path.join(__dirname, 'fixtures', 'no-such-connector'), args: [], env: {} },
      { label: 'test', key: 'missing' }
    )
    await expect(failed).rejects.toMatchObject({ reason: 'spawn' })
  })

  it('closes once, however often it is asked, and refuses requests afterwards', async () => {
    const client = await start('normal')
    const exit = exitOf(client)
    await Promise.all([client.close(), client.close()])
    expect(await exit).toEqual({ code: 0, signal: null })
    await expect(client.request('vorn/hello', HELLO_PARAMS, 5_000)).rejects.toMatchObject({
      reason: 'closed'
    })
  })

  it.runIf(process.platform !== 'win32')(
    'kills a child that ignores both the end of its input and SIGTERM',
    async () => {
      const client = await start('stubborn', {
        closeTimings: { termAfterMs: 50, killAfterMs: 200 }
      })
      // Answering the hello proves the SIGTERM handler is in place, even on a loaded machine.
      await client.request('vorn/hello', HELLO_PARAMS, 5_000)
      const exit = exitOf(client)
      await client.close()
      expect(client.exited).toBe(true)
      expect(await exit).toEqual({ code: null, signal: 'SIGKILL' })
    }
  )
})
