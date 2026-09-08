import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Fastify, { type FastifyInstance } from 'fastify'
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { InstalledConnectorPack, TerminalSession } from '@vornrun/shared/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const pack = { current: undefined as InstalledConnectorPack | undefined }
const hosts = new Map<string, { extensionId: string; projectPath: string; token: string }>()
const grants = new Map<
  string,
  { extensionId: string; paneId: string; sessionId: string; projectPath: string }
>()
const sessions: TerminalSession[] = []
const wrote: Array<{ id: string; data: string }> = []
const renamed: Array<{ id: string; name: string; byPerson: boolean }> = []
const saved: Array<{ id: string; name: string }> = []
let output: string[] = ['$ yarn test', 'ok']

vi.mock('../packages/server/src/connectors/packs', async (importOriginal) => ({
  // The page file types come from the pack gate itself, so the two cannot drift.
  WEB_FILE_TYPES: (await importOriginal<typeof import('../packages/server/src/connectors/packs')>())
    .WEB_FILE_TYPES,
  installedPack: (id: string) => (pack.current?.id === id ? pack.current : undefined)
}))

vi.mock('../packages/server/src/extensions/hosts', () => ({
  hostByToken: (id: string, token: string) =>
    [...hosts.values()].find((host) => host.extensionId === id && host.token === token)
}))

vi.mock('../packages/server/src/extensions/panes', () => ({
  grantFor: (nonce: string) => grants.get(nonce)
}))

vi.mock('../packages/server/src/extensions/selection', () => ({
  requestSelection: async () => 'the highlighted text'
}))

vi.mock('../packages/server/src/extensions/usage', () => ({
  usageFor: () => ({ contextTokens: 1000, cacheHitRate: 0.9 })
}))

vi.mock('../packages/server/src/git-utils', () => ({
  getGitDiffText: () => 'diff --git a/x b/x\n',
  getGitStatusPorcelain: () => ' M src/index.ts\n'
}))

vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: {
    getLiveSessions: () => sessions,
    getOutput: () => output,
    writeToPty: (id: string, data: string) => wrote.push({ id, data }),
    renameSession: (id: string, name: string, byPerson = true) =>
      renamed.push({ id, name, byPerson })
  }
}))

const { registerExtensionBridge, registerExtensionPages } =
  await import('../packages/server/src/extensions/bridge')

const temps: string[] = []
let app: FastifyInstance
let pages: FastifyInstance

function packDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-bridge-pack-'))
  temps.push(dir)
  mkdirSync(join(dir, 'web', 'report'), { recursive: true })
  writeFileSync(join(dir, 'web', 'report', 'index.html'), '<h1>Report</h1>')
  writeFileSync(join(dir, 'web', 'report', 'app.js'), 'export const a = 1')
  writeFileSync(join(dir, 'web', 'report', 'notes.node'), 'native')
  writeFileSync(join(dir, 'index.js'), 'the entry')
  return dir
}

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 's1',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    status: 'running',
    createdAt: 0,
    pid: 1,
    ...over
  } as TerminalSession
}

beforeEach(async () => {
  const path = packDir()
  pack.current = {
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    path,
    installedAt: 0,
    bytes: 0,
    triggers: [],
    actions: [],
    env: [],
    contributes: { panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }] },
    permissions: ['git.read', 'terminal.read', 'terminal.selection', 'terminal.send', 'agent.usage']
  }
  hosts.clear()
  hosts.set('review', { extensionId: 'review', projectPath: '/work/vorn', token: 'right-token' })
  grants.clear()
  grants.set('nonce-1', {
    extensionId: 'review',
    paneId: 'report',
    sessionId: 's1',
    projectPath: '/work/vorn'
  })
  sessions.length = 0
  sessions.push(session())
  wrote.length = 0
  renamed.length = 0
  saved.length = 0
  output = ['$ yarn test', 'ok']

  const deps = {
    frameAncestors: () => ['http://127.0.0.1:7777'],
    sessionRenamed: (id: string, name: string) => saved.push({ id, name })
  }
  app = Fastify()
  registerExtensionBridge(app, deps)
  await app.ready()
  // Pages live on their own origin, so they are registered on their own instance.
  pages = Fastify()
  registerExtensionPages(pages, deps)
  await pages.ready()
})

afterEach(async () => {
  await app.close()
  await pages.close()
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

const call = (method: string, body: unknown = { sessionId: 's1' }, token = 'right-token') =>
  app.inject({
    method: 'POST',
    url: `/extensions/review/bridge/${method}`,
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    payload: body as never,
    remoteAddress: '127.0.0.1'
  })

// The server cannot import the SDK at runtime, so the two tables are compared here.
describe('what each method costs', () => {
  it('charges what the SDK gates on, method for method', async () => {
    const { METHOD_PERMISSIONS } = await import('../packages/server/src/extensions/bridge')
    const { HOST_PERMISSIONS } = await import('../packages/connector-sdk/src/define')
    expect(METHOD_PERMISSIONS).toEqual(HOST_PERMISSIONS)
  })
})

describe('who the bridge answers', () => {
  it('refuses a caller it has no token for', async () => {
    const answer = await call('status', { sessionId: 's1' }, 'wrong-token')
    expect(answer.statusCode).toBe(401)
  })

  it('refuses a call from off this machine', async () => {
    const answer = await app.inject({
      method: 'POST',
      url: '/extensions/review/bridge/status',
      headers: { authorization: 'Bearer right-token' },
      payload: { sessionId: 's1' },
      remoteAddress: '10.0.0.4'
    })
    expect(answer.statusCode).toBe(403)
    expect(answer.body).toContain('Local machine only')
  })

  // The wording matches the check's stub, so an extension meets one rule rather than two.
  it('refuses a method the manifest never asked for', async () => {
    const answer = await call('rename', { sessionId: 's1', name: 'Anything' })
    expect(answer.statusCode).toBe(403)
    expect(answer.body).toBe('this extension does not ask for card.rename')
    expect(renamed).toEqual([])
  })

  it('refuses a session belonging to another project', async () => {
    sessions.push(session({ id: 's2', projectPath: '/work/other' }))
    const answer = await call('status', { sessionId: 's2' })
    expect(answer.statusCode).toBe(403)
    expect(answer.body).toContain('another project')
  })

  it('refuses a session that is not running', async () => {
    expect((await call('status', { sessionId: 'gone' })).statusCode).toBe(404)
  })

  it('serves no method it does not have', async () => {
    expect((await call('exec')).statusCode).toBe(404)
  })

  // Neither header is one a page can forge, and the extension's own process sends neither.
  it('refuses a call a browser says came from another site', async () => {
    const bySite = await app.inject({
      method: 'POST',
      url: '/extensions/review/bridge/status',
      headers: {
        authorization: 'Bearer right-token',
        'content-type': 'application/json',
        'sec-fetch-site': 'cross-site'
      },
      payload: { sessionId: 's1' },
      remoteAddress: '127.0.0.1'
    })
    expect(bySite.statusCode).toBe(403)
    expect(bySite.body).toContain('another site')

    const byOrigin = await app.inject({
      method: 'POST',
      url: '/extensions/review/bridge/status',
      headers: {
        authorization: 'Bearer right-token',
        'content-type': 'application/json',
        origin: 'https://elsewhere.example'
      },
      payload: { sessionId: 's1' },
      remoteAddress: '127.0.0.1'
    })
    expect(byOrigin.statusCode).toBe(403)
  })
})

describe('what the bridge answers with', () => {
  it('answers a read with its result', async () => {
    const status = await call('status')
    expect(status.statusCode).toBe(200)
    expect(status.json()).toEqual({ result: ' M src/index.ts\n' })

    expect((await call('diff')).json()).toEqual({ result: 'diff --git a/x b/x\n' })
    expect((await call('output')).json()).toEqual({ result: '$ yarn test\nok' })
    expect((await call('selection')).json()).toEqual({ result: 'the highlighted text' })
    expect((await call('usage')).json()).toEqual({
      result: { contextTokens: 1000, cacheHitRate: 0.9 }
    })
  })

  it('answers a write with nothing at all', async () => {
    const answer = await call('send', { sessionId: 's1', text: 'yarn test\r' })
    expect(answer.statusCode).toBe(204)
    expect(answer.body).toBe('')
    expect(wrote).toEqual([{ id: 's1', data: 'yarn test\r' }])
  })

  // A line has no length of its own, so a cap counted in lines is no cap at all.
  it('answers output held to a size a reader can take, keeping the end', async () => {
    output = ['x'.repeat(400 * 1024), 'the last line']
    const answer = await call('output')
    const text = answer.json<{ result: string }>().result
    expect(text.length).toBe(256 * 1024)
    expect(text.endsWith('the last line')).toBe(true)
  })

  // Most cards arrive with a name filled in, so only a rename a person asked for counts.
  it('renames a card, and refuses one its person named', async () => {
    pack.current!.permissions = [...(pack.current!.permissions ?? []), 'card.rename']

    const allowed = await call('rename', { sessionId: 's1', name: 'Checks' })
    expect(allowed.statusCode).toBe(204)
    expect(renamed).toEqual([{ id: 's1', name: 'Checks', byPerson: false }])
    expect(saved).toEqual([{ id: 's1', name: 'Checks' }])

    sessions[0].renamedByPerson = true
    const refused = await call('rename', { sessionId: 's1', name: 'Something else' })
    expect(refused.statusCode).toBe(403)
    expect(refused.body).toContain('named by the person')
    expect(renamed).toHaveLength(1)
  })

  it('refuses a write missing what it writes', async () => {
    expect((await call('send', { sessionId: 's1' })).statusCode).toBe(500)
    expect(wrote).toEqual([])
  })
})

describe('a pane page and the nonce that proves it', () => {
  const page = (path: string, nonce = 'nonce-1') =>
    pages.inject({
      method: 'GET',
      url: `/extensions/review/pane/report/${nonce}/${path}`,
      remoteAddress: '127.0.0.1'
    })

  it('serves the page the pane names', async () => {
    const answer = await page('')
    expect(answer.statusCode).toBe(200)
    expect(answer.body).toBe('<h1>Report</h1>')
    expect(answer.headers['content-type']).toContain('text/html')
    expect(answer.headers['content-security-policy']).toContain(
      'frame-ancestors http://127.0.0.1:7777'
    )
    expect(answer.headers['referrer-policy']).toBe('no-referrer')
    expect(answer.headers['cross-origin-opener-policy']).toBe('same-origin')
    expect(answer.headers['x-content-type-options']).toBe('nosniff')
  })

  it('serves what the page is made of', async () => {
    expect((await page('app.js')).statusCode).toBe(200)
  })

  it('serves nothing that is not a page', async () => {
    expect((await page('notes.node')).statusCode).toBe(404)
  })

  it('serves nothing outside the pane directory', async () => {
    expect((await page('../../index.js')).statusCode).toBe(404)
    expect((await page('..%2f..%2findex.js')).statusCode).toBe(404)
  })

  it('serves nothing through a link out of the directory', async () => {
    const outside = mkdtempSync(join(tmpdir(), 'vorn-bridge-outside-'))
    temps.push(outside)
    writeFileSync(join(outside, 'secret.txt'), 'not yours')
    symlinkSync(join(outside, 'secret.txt'), join(pack.current!.path, 'web', 'report', 'out.txt'))
    expect((await page('out.txt')).statusCode).toBe(404)
  })

  it('serves nothing for a nonce it never minted', async () => {
    expect((await page('', 'made-up')).statusCode).toBe(404)
    grants.delete('nonce-1')
    expect((await page('')).statusCode).toBe(404)
  })

  // A page speaks for the pane it was opened as, never for a session it names itself.
  it('answers a page on its own pane session, not the one it asks for', async () => {
    sessions.push(session({ id: 's2', projectPath: '/work/vorn' }))
    const answer = await pages.inject({
      method: 'POST',
      url: '/extensions/review/pane/report/nonce-1/bridge/send',
      headers: { 'content-type': 'application/json' },
      payload: { sessionId: 's2', text: 'hello' },
      remoteAddress: '127.0.0.1'
    })
    expect(answer.statusCode).toBe(204)
    expect(wrote).toEqual([{ id: 's1', data: 'hello' }])
  })

  it('is served by nothing on the origin the app is on', async () => {
    const onApp = await app.inject({
      method: 'GET',
      url: '/extensions/review/pane/report/nonce-1/',
      remoteAddress: '127.0.0.1'
    })
    expect(onApp.statusCode).toBe(404)
  })

  it('refuses a page bridge call once the pane has closed', async () => {
    grants.delete('nonce-1')
    const answer = await pages.inject({
      method: 'POST',
      url: '/extensions/review/pane/report/nonce-1/bridge/status',
      headers: { 'content-type': 'application/json' },
      payload: {},
      remoteAddress: '127.0.0.1'
    })
    expect(answer.statusCode).toBe(401)
  })
})
