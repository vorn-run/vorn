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
 * Attaching to a session vornd holds, and the one reconnect rule: a client
 * continues from its cursor without a snapshot while every record after it is
 * retained, across a vornd restart included (RC-T10, TP-T24); otherwise it gets
 * a snapshot and the reason. Real vornd and session holder; vornd is killed
 * with SIGKILL, as a crash would.
 *
 * Runs in `yarn test:conformance`, which builds both binaries; skipped
 * otherwise.
 */
describe.runIf(vorndSessionsAvailable)('attaching through vornd', () => {
  let server: Awaited<ReturnType<typeof upstream>>
  let dir: ReturnType<typeof home>
  let vornd: Vornd
  let holder: number | null = null
  const clients: BytesClient[] = []

  async function client(port = vornd.port): Promise<BytesClient> {
    const c = new BytesClient()
    await c.connect(port)
    clients.push(c)
    return c
  }

  beforeAll(async () => {
    server = await upstream()
    dir = home()
    vornd = await Vornd.start(server.port, dir.dir)
    holder = await vornd.sessiondPid()
  }, 30_000)

  afterAll(async () => {
    for (const c of clients) c.close()
    await vornd.kill()
    killPid(holder)
    server.close()
    dir.remove()
  })

  it('gives a client with no cursor a snapshot, and one that resumes the rest', async () => {
    const c = await client()
    const id = await c.spawn(['sh', '-c', 'echo first; read x; echo second; sleep 5'])
    await until('output', async () => {
      const s = await c
        .call<{ data: string }>('terminal:readScrollback', { id })
        .catch(() => ({ data: '' }))
      return s.data.includes('first')
    })
    const a = await c.attach(id)
    expect(a.continued).toBe(false)
    expect(await c.text()).toContain('first')

    // A second connection, resuming from the first one's cursor, continues.
    const resumed = await client()
    resumed.cursor = c.cursor
    await resumed.attach(id, true).then((r) => expect(r.continued).toBe(true))

    // A cursor from another epoch is no resume token.
    const stale = await client()
    stale.cursor = { ...c.cursor!, epoch: c.cursor!.epoch + 1 }
    const s = await stale.attach(id, true)
    expect(s.continued).toBe(false)
    expect(s.resync).toBe('wrongEpoch')

    c.notify('terminal:write', { id, data: 'x\r' })
    await until('the rest', async () => (await c.text()).includes('second'))
    expect(c.broken).toBeNull()
  })

  it('continues across a vornd restart without a snapshot, each line once (RC-T10)', async () => {
    const c = await client()
    const id = await c.spawn([
      'sh',
      '-c',
      'read go; i=0; while [ $i -lt 60 ]; do echo "line $i"; i=$((i+1)); sleep 0.05; done; sleep 10'
    ])
    await c.attach(id)
    c.notify('terminal:write', { id, data: 'go\r' })
    await until('some lines', async () => (await c.text()).includes('line 5'))

    // vornd dies mid-output; the program goes on printing into sessiond's ring.
    await vornd.kill()
    c.close()
    await new Promise((r) => setTimeout(r, 1000))
    vornd = await Vornd.start(server.port, dir.dir)
    await until('the session to be live again', async () =>
      (await vornd.report()).sessions.some((s) => s.session === id && s.state === 'live')
    )

    await c.connect(vornd.port)
    const a = await c.attach(id, true)
    expect(a.continued).toBe(true)
    await until('the last line', async () => (await c.text()).includes('line 59'))
    expect(c.broken).toBeNull()
    const lines = (await c.text()).split('\n').filter((l) => /^line \d+$/.test(l))
    expect(lines).toEqual(Array.from({ length: 60 }, (_, i) => `line ${i}`))
  }, 60_000)
})
