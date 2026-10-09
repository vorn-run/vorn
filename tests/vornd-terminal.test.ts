import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import {
  BytesClient,
  Vornd,
  home,
  killPid,
  until,
  upstream,
  vorndSessionsAvailable
} from './helpers/vornd-sessions'

/**
 * The terminal calls for a session vornd holds, answered by vornd itself and
 * never forwarded: attach, write, resize, readOutput and readScrollback, and
 * the data, resized and exit notifications. Real vornd, real session holder,
 * real shells; the client is xterm.js wired as the renderer wires it.
 *
 * Runs in `yarn test:conformance`, which builds both binaries; skipped
 * otherwise.
 */
describe.runIf(vorndSessionsAvailable)('terminal calls through vornd', () => {
  let server: Awaited<ReturnType<typeof upstream>>
  let dir: ReturnType<typeof home>
  let vornd: Vornd
  const clients: BytesClient[] = []

  async function client(): Promise<BytesClient> {
    const c = new BytesClient()
    await c.connect(vornd.port)
    clients.push(c)
    return c
  }

  beforeAll(async () => {
    server = await upstream()
    dir = home()
    vornd = await Vornd.start(server.port, dir.dir)
  }, 30_000)

  afterAll(async () => {
    for (const c of clients) c.close()
    const holder = await vornd.sessiondPid().catch(() => null)
    await vornd.kill()
    killPid(holder)
    server.close()
    dir.remove()
  })

  it('writes, resizes and reads a session it holds, and tells every client it ended', async () => {
    const c = await client()
    const id = await c.spawn([
      'sh',
      '-c',
      'read line; echo "got-$line"; stty size; sleep 0.3; exit 3'
    ])
    const a = await c.attach(id)
    expect(a.replies).toBe('vornd')
    const other = await client()
    other.session = id

    c.notify('terminal:resize', { id, cols: 100, rows: 30 })
    await until('the resize to reach the client', () => c.term.cols === 100 && c.term.rows === 30)
    c.notify('terminal:write', { id, data: 'hello\r' })
    await until('the program to answer', async () => (await c.text()).includes('got-hello'))
    // The program saw the size as a stream record, at its place.
    await until('stty to report it', async () => (await c.text()).includes('30 100'))

    const output = await c.call<string[]>('terminal:readOutput', { id, lines: 10 })
    expect(output.join('\n')).toContain('got-hello')
    const scrollback = await c.call<{ data: string }>('terminal:readScrollback', { id })
    expect(scrollback.data).toContain('got-hello')

    // The attached client hears the exit after the last bytes; the other one too.
    await until('the exit', () => c.exits.length === 1 && other.exits.length === 1)
    expect(c.exits).toEqual([3])
    expect(other.exits).toEqual([3])
    expect(c.broken).toBeNull()
  })

  it('answers each terminal query exactly once, with three clients attached (TP-T12)', async () => {
    const first = await client()
    // Waits for a go, then asks for device attributes and reports what came back.
    const id = await first.spawn([
      'sh',
      '-c',
      'read go; stty raw -echo; printf "\\033[c"; sleep 1; dd bs=256 count=1 2>/dev/null | od -An -c | tr -s " "; stty sane; sleep 2'
    ])
    await first.attach(id)
    const second = await client()
    const third = await client()
    await second.attach(id)
    await third.attach(id)

    first.notify('terminal:write', { id, data: 'go\r' })
    await until('the program to report the replies', async () =>
      (await first.text()).includes(' c')
    )
    const report = (await first.text()).split('\n').find((l) => l.includes('033'))!
    // One escape sequence came back: vornd's, and none of the three clients'.
    expect(report.match(/033/g)).toHaveLength(1)
    expect(report).toMatch(/033 \[ \? .* c/)
  })

  it('answers the attach and the writes of a session nothing holds, forwarding neither', async () => {
    const c = await client()
    expect(await c.call('terminal:attach', { id: 'not-held-by-vornd' })).toEqual({
      data: '',
      seq: 0,
      live: false
    })
    // The stand-in server answers nothing, so only vornd can answer.
    const answered = await Promise.race([
      c.call('terminal:write', { id: 'not-held-by-vornd', data: 'x' }).then(
        () => true,
        () => true
      ),
      new Promise((r) => setTimeout(() => r(false), 2_000))
    ])
    expect(answered).toBe(true)
  })
})
