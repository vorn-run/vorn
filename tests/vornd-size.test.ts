import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import {
  BytesClient,
  Vornd,
  home,
  killPid,
  until,
  vorndSessionsAvailable
} from './helpers/vornd-sessions'

/**
 * The size rule through the real vornd and session holder, as the desktop and
 * a phone use it over the WebSocket. Which connection is the desktop's is
 * decided by the launch token it opened with, never by anything it says.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`);
 * skipped otherwise.
 */
describe.runIf(vorndSessionsAvailable)('the size of a session vornd holds', () => {
  const TOKEN = 'desktop-launch-token'
  let dir: ReturnType<typeof home>
  let vornd: Vornd
  const clients: BytesClient[] = []

  async function client(token: string): Promise<BytesClient> {
    const c = new BytesClient()
    await c.connect(vornd.port, token)
    clients.push(c)
    return c
  }

  const pause = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

  beforeAll(async () => {
    dir = home()
    vornd = await Vornd.start(dir.dir, TOKEN)
  }, 30_000)

  afterAll(async () => {
    for (const c of clients) c.close()
    const holder = await vornd.sessiondPid().catch(() => null)
    await vornd.kill()
    killPid(holder)
    dir.remove()
  })

  it('stays still while looked at, follows typing once, and the desktop wins ties', async () => {
    const desktop = await client(TOKEN)
    // A phone over the tunnel, presenting a device token rather than the desktop's.
    const phone = await client(await vornd.deviceToken('phone'))
    const id = await desktop.spawn(['cat'], 100, 30)
    await desktop.attach(id)
    await phone.attach(id)
    desktop.notify('terminal:viewport', { id, cols: 120, rows: 40 })
    phone.notify('terminal:viewport', { id, cols: 50, rows: 30 })

    // Looking: presence, focus reports and scrolling, no typing.
    for (let i = 0; i < 5; i++) {
      for (const state of ['active', 'away', 'watching'])
        phone.notify('terminal:presence', { id, state })
      phone.notify('terminal:write', { id, data: '\x1b[I\x1b[<64;3;3M' })
    }
    await pause(600)
    expect(phone.resized).toEqual([])
    expect(phone.term.cols).toBe(100)

    // Both typing, alternately: one resize, and it is the desktop's.
    for (let i = 0; i < 8; i++) {
      ;(i % 2 ? phone : desktop).notify('terminal:write', { id, data: `${i}` })
      await pause(250)
    }
    await until('the resize', () => phone.resized.length > 0)
    await pause(600)
    expect(phone.resized.map((r) => [r.cols, r.rows, r.owner, r.reason])).toEqual([
      [120, 40, desktop.name, 'input']
    ])
    expect(desktop.resized).toHaveLength(1)
    await until('both at the new size', () => desktop.term.cols === 120 && phone.term.cols === 120)
  }, 20_000)

  it('fits a new terminal to the pane that opened it on first draw', async () => {
    const desktop = await client(TOKEN)
    const phone = await client(await vornd.deviceToken('phone'))
    const { id } = await desktop.call<{ id: string }>('shell:create', dir.dir)
    // As the renderer shows it: attach, report the pane, and again when told
    // to, for a pane that attached while the session started.
    const views = new Map<BytesClient, { view: object; resyncs: number }>([
      [phone, { view: { cols: 50, rows: 30 }, resyncs: -1 }],
      [desktop, { view: { pane: 1, cols: 200, rows: 50 }, resyncs: -1 }]
    ])
    const show = async (): Promise<void> => {
      for (const [c, v] of views) {
        if (c.resyncs.length === v.resyncs) continue
        v.resyncs = c.resyncs.length
        await c.attach(id)
        c.notify('terminal:viewport', { id, ...v.view })
      }
    }
    await show()

    await until('the pane size', async () => {
      await show()
      return desktop.resized.length > 0
    })
    await pause(600)
    expect(desktop.resized.map((r) => [r.cols, r.rows, r.owner, r.reason])).toEqual([
      [200, 50, `${desktop.name}:1`, 'launch']
    ])
    await until('both at the pane size', () => desktop.term.cols === 200 && phone.term.cols === 200)
    await desktop.call('terminal:kill', id)
    await until('the shell to exit', () => desktop.exits.length > 0)
  }, 20_000)
})
