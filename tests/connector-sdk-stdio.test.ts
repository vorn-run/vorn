import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import path from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MAX_FRAME_BYTES } from '../packages/connector-sdk/src/index'

const REPO = path.join(__dirname, '..')
const FIXTURE = path.join(__dirname, 'fixtures', 'sdk-connector.ts')
const HELLO = {
  jsonrpc: '2.0',
  id: 1,
  method: 'vorn/hello',
  params: { protocols: [1], host: { name: 'vörn ✓', version: 'test' } }
}

interface Served {
  child: ChildProcessWithoutNullStreams
  lines: string[]
  stderr: () => string
  exited: Promise<number | null>
  send(message: unknown): void
}

const started: ChildProcessWithoutNullStreams[] = []

afterEach(() => {
  for (const child of started.splice(0)) child.kill('SIGKILL')
})

/** The fixture connector run the way Vorn runs a checkout, with every stdout line kept. */
function serve(): Served {
  const child = spawn(process.execPath, ['--import', 'tsx', FIXTURE], { cwd: REPO })
  started.push(child)
  const lines: string[] = []
  let out = ''
  let err = ''
  child.stdout.setEncoding('utf8').on('data', (chunk: string) => {
    out += chunk
    for (let end = out.indexOf('\n'); end !== -1; end = out.indexOf('\n')) {
      lines.push(out.slice(0, end))
      out = out.slice(end + 1)
    }
  })
  child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
    err += chunk
  })
  child.stdin.on('error', () => {})
  return {
    child,
    lines,
    stderr: () => err,
    exited: new Promise((resolve) => child.once('exit', resolve)),
    send: (message) => child.stdin.write(`${JSON.stringify(message)}\n`)
  }
}

const replies = (served: Served) => served.lines.map((line) => JSON.parse(line))

describe('a connector served on stdio', () => {
  it('keeps stdout for replies and sends what it prints to stderr', async () => {
    const served = serve()
    served.send(HELLO)
    served.send({
      jsonrpc: '2.0',
      id: 2,
      method: 'action/run',
      params: { action: 'echo', args: { text: 'hi', count: '3' } }
    })
    await vi.waitFor(() => expect(served.lines).toHaveLength(2), { timeout: 20_000 })

    expect(replies(served)).toEqual([
      {
        jsonrpc: '2.0',
        id: 1,
        result: {
          protocol: 1,
          sdk: { name: '@vornrun/connector-sdk', version: expect.any(String) },
          connector: { id: 'sdk-fixture', version: '1.0.0', kind: 'connector' }
        }
      },
      { jsonrpc: '2.0', id: 2, result: { text: 'hi', count: 3 } }
    ])
    expect(served.stderr()).toContain('booting')
    expect(served.stderr()).toContain('echoing hi')
  }, 30_000)

  it('skips a line that is not JSON and keeps serving', async () => {
    const served = serve()
    served.child.stdin.write('not json\n\n')
    served.send(HELLO)
    await vi.waitFor(() => expect(served.lines).toHaveLength(1), { timeout: 20_000 })
    expect(replies(served)[0]).toMatchObject({ id: 1, result: { protocol: 1 } })
    expect(served.stderr()).toContain('sdk-fixture: skipped a line that is not JSON')
  }, 30_000)

  it('reads a message split mid-character across writes, and one ending in CRLF', async () => {
    const served = serve()
    const bytes = Buffer.from(`${JSON.stringify(HELLO)}\r\n`)
    const inside = bytes.indexOf(Buffer.from('ö')) + 1
    served.child.stdin.write(bytes.subarray(0, inside))
    await new Promise((resolve) => setTimeout(resolve, 50))
    served.child.stdin.write(bytes.subarray(inside))
    served.send({
      jsonrpc: '2.0',
      id: 2,
      method: 'action/run',
      params: { action: 'echo', args: { text: 'é ✓' } }
    })
    await vi.waitFor(() => expect(served.lines).toHaveLength(2), { timeout: 20_000 })
    expect(replies(served)[1]).toEqual({ jsonrpc: '2.0', id: 2, result: { text: 'é ✓' } })
  }, 30_000)

  it('finishes the calls in flight before exiting once its input ends', async () => {
    const served = serve()
    served.send(HELLO)
    served.send({
      jsonrpc: '2.0',
      id: 2,
      method: 'action/run',
      params: { action: 'wait', args: { ms: 300 } }
    })
    await vi.waitFor(() => expect(served.lines).toHaveLength(1), { timeout: 20_000 })
    served.child.stdin.end()

    expect(await served.exited).toBe(0)
    expect(replies(served)[1]).toEqual({ jsonrpc: '2.0', id: 2, result: { waited: 300 } })
  }, 30_000)

  it('ends rather than buffer a message past the frame limit', async () => {
    const served = serve()
    served.send(HELLO)
    await vi.waitFor(() => expect(served.lines).toHaveLength(1), { timeout: 20_000 })
    served.child.stdin.write(Buffer.alloc(MAX_FRAME_BYTES + 1, 'x'))

    expect(await served.exited).toBe(1)
    expect(served.stderr()).toContain(`sdk-fixture: a message was over ${MAX_FRAME_BYTES} bytes`)
  }, 30_000)
})
