/**
 * Extensions hosted by vornd, on a real server and its vornd: a pack installed
 * in the data directory is listed and activated on a session, its child is
 * started with a token of its own, its footer and link handler are answered,
 * its pane page is served, its bridge takes only its own token, and a child
 * that dies is started again.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`), on
 * a Unix.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  Watcher,
  realServers,
  removeRealServerDirs,
  repository,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type RealServer
} from './helpers/real-server'

spawnsRealServers()

const CHILD = fs.readFileSync(path.join(__dirname, 'fixtures', 'extension-pack.mjs'), 'utf8')

interface Reading {
  extensionId: string
  items: Array<{ label: string; value: string }>
}

/** A pack as an install leaves it: its files under its version, and which version is current. */
function install(dataDir: string, id: string): void {
  const root = path.join(dataDir, 'connectors', id)
  const dir = path.join(root, '1.0.0')
  fs.mkdirSync(path.join(dir, 'web'), { recursive: true })
  fs.writeFileSync(path.join(dir, 'index.js'), CHILD)
  fs.writeFileSync(path.join(dir, 'package.json'), '{"type":"module"}')
  fs.writeFileSync(path.join(dir, 'web', 'index.html'), `<p>${id} pane</p>`)
  const manifest = {
    id,
    name: id.toUpperCase(),
    version: '1.0.0',
    kind: 'extension',
    protocol: 1,
    permissions: ['git.read', 'terminal.selection'],
    contributes: {
      panes: [{ id: 'page', title: 'Page', web: 'web/index.html' }],
      footers: [{ id: 'who', every: 5 }],
      linkHandlers: [{ id: 'ticket', pattern: `${id.toUpperCase()}-\\d+` }]
    }
  }
  fs.writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify(manifest))
  fs.writeFileSync(
    path.join(root, 'current.json'),
    JSON.stringify({ version: '1.0.0', installedAt: new Date().toISOString() })
  )
}

const valueOf = (reading: Reading | undefined, label: string): string =>
  reading?.items.find((i) => i.label === label)?.value ?? ''

describe.skipIf(!runnable)('extensions through vornd', () => {
  let server: RealServer
  let through: Watcher
  let session: TerminalSession

  const readingOf = async (id: string): Promise<Reading | undefined> => {
    const readings = await through.result<Reading[]>('extension:footerItems', {
      sessionId: session.id
    })
    return readings.find((r) => r.extensionId === id && r.items.length > 0)
  }

  const bridge = (host: string, token: string, method: string): Promise<Response> =>
    fetch(`${host}/${method}`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ sessionId: session.id })
    })

  beforeAll(async () => {
    try {
      server = await startRealServer()
      install(server.dirs.data, 'demo')
      install(server.dirs.data, 'other')
      const project = path.join(server.dirs.work, 'proj')
      repository(project)
      through = await Watcher.open(server.vornd)
      await through.result('config:load')
      session = await through.result<TerminalSession>('shell:create', project)
    } catch (err) {
      const log = (server ?? realServers.at(-1))?.log.join('') ?? ''
      throw new Error(`${(err as Error).message}\n${log.slice(-4000)}`, {
        cause: err
      })
    }
  }, 240_000)

  afterAll(async () => {
    through?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  }, 120_000)

  it('lists the installed packs and activates them on the session', async () => {
    const list = await through.result<Array<{ id: string }>>('extension:list')
    expect(list.map((p) => p.id)).toEqual(['demo', 'other'])
    const states = await through.result<Array<{ extensionId: string; footers: string[] }>>(
      'extension:activation',
      { sessionId: session.id }
    )
    expect(states.map((s) => [s.extensionId, s.footers])).toEqual([
      ['demo', ['who']],
      ['other', ['who']]
    ])
  })

  it('reads each footer from a child holding a token of its own', async () => {
    await until(
      'both footers',
      async () => !!(await readingOf('demo')) && !!(await readingOf('other'))
    )
    const demo = await readingOf('demo')
    const other = await readingOf('other')
    expect(valueOf(demo, 'host')).toBe(`http://127.0.0.1:${server.vornd}/extensions/demo/bridge`)
    expect(valueOf(demo, 'bridge')).toBe('200')
    expect(valueOf(demo, 'token')).toHaveLength(43)
    expect(valueOf(demo, 'token')).not.toBe(valueOf(other, 'token'))
    expect(valueOf(demo, 'pid')).not.toBe(valueOf(other, 'pid'))
  })

  it("refuses one extension's token on another's bridge", async () => {
    const demo = await readingOf('demo')
    const other = await readingOf('other')
    const host = valueOf(demo, 'host')
    expect((await bridge(host, valueOf(demo, 'token'), 'status')).status).toBe(200)
    expect((await bridge(host, valueOf(other, 'token'), 'status')).status).toBe(401)
    expect((await bridge(host, 'made-up', 'status')).status).toBe(401)
  })

  it('serves a pane page that only the app may frame', async () => {
    const grant = await through.result<{ nonce: string; url: string }>('extension:openPane', {
      extensionId: 'demo',
      paneId: 'page',
      sessionId: session.id
    })
    const page = await fetch(grant.url)
    expect(page.status).toBe(200)
    expect(await page.text()).toBe('<p>demo pane</p>')
    expect(page.headers.get('content-security-policy')).toContain(
      `frame-ancestors http://127.0.0.1:${server.vornd}`
    )
    expect(await through.result('extension:closePane', { nonce: grant.nonce })).toEqual({
      closed: true
    })
    expect((await fetch(grant.url)).status).toBe(404)
  })

  it('matches links and opens the pane their handler names', async () => {
    const links = await through.result<Array<{ extensionId: string; handlerId: string }>>(
      'extension:matchLinks',
      { sessionId: session.id, text: 'see DEMO-12' }
    )
    expect(links.map((l) => [l.extensionId, l.handlerId])).toEqual([['demo', 'ticket']])
    const opened = await through.result<{ openedPane?: { url: string } }>('extension:runHandler', {
      extensionId: 'demo',
      handlerId: 'ticket',
      sessionId: session.id,
      url: 'DEMO-12'
    })
    expect(opened.openedPane?.url).toContain('/extensions/demo/pane/page/')
    expect(
      await through.result('extension:runHandler', {
        extensionId: 'other',
        handlerId: 'ticket',
        sessionId: session.id,
        url: 'OTHER-7'
      })
    ).toEqual({})
  })

  it("asks the session's window for its selection", async () => {
    const demo = await readingOf('demo')
    const asked = bridge(valueOf(demo, 'host'), valueOf(demo, 'token'), 'selection')
    await until(
      'the selection request',
      () => through.toldBy('extension:selectionRequest').length > 0
    )
    const [request] = through.toldBy('extension:selectionRequest') as Array<{
      requestId: number
      sessionId: string
    }>
    expect(request.sessionId).toBe(session.id)
    through.notify('extension:selectionResult', { requestId: request.requestId, text: 'picked' })
    const res = await asked
    expect(res.status).toBe(200)
    expect(await res.json()).toEqual({ result: 'picked' })
  })

  it('starts a child that died again, with a new token', async () => {
    const before = await readingOf('demo')
    process.kill(Number(valueOf(before, 'pid')), 'SIGKILL')
    await until('the child to start again', async () => {
      const now = await readingOf('demo')
      return !!now && valueOf(now, 'pid') !== valueOf(before, 'pid')
    })
    const after = await readingOf('demo')
    expect(valueOf(after, 'token')).not.toBe(valueOf(before, 'token'))
    expect(valueOf(after, 'bridge')).toBe('200')
    const host = valueOf(after, 'host')
    expect((await bridge(host, valueOf(before, 'token'), 'status')).status).toBe(401)
  }, 60_000)

  it('reports its extension hosts in its health check', async () => {
    const health = (await (
      await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    ).json()) as { extensions: { hosts?: number } }
    expect(health.extensions).toMatchObject({ hosts: 2 })
  })
})
