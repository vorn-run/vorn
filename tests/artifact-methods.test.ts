import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const h = vi.hoisted(() => ({
  handlers: new Map<string, (params: never) => unknown>(),
  listeners: new Map<string, (...args: unknown[]) => void>(),
  sessions: [] as Array<Record<string, unknown>>,
  writes: [] as Array<{ id: string; data: string }>,
  broadcasts: [] as Array<{ method: string; params: unknown }>,
  bridge: vi.fn(async () => ({}))
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/ws-handler', () => ({
  registerMethod: (method: string, handler: (params: never) => unknown) =>
    h.handlers.set(method, handler)
}))
vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: {
    getActiveSessions: () => h.sessions,
    writeToPty: (id: string, data: string) => h.writes.push({ id, data }),
    on: (event: string, fn: (...args: unknown[]) => void) => h.listeners.set(event, fn)
  }
}))
vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: {
    broadcast: (method: string, params: unknown) => h.broadcasts.push({ method, params })
  }
}))
vi.mock('../packages/server/src/browser-bridge', () => ({
  browserBridge: { request: h.bridge }
}))

import { IPC } from '@vornrun/shared/types'
import { initTestDatabase, getDataDir } from '../packages/server/src/database'
import { registerArtifactMethods, sealGateDrafts } from '../packages/server/src/artifacts/methods'
import { publishGateArtifact } from '../packages/server/src/artifacts/service'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const call = (method: string, params: any): any => {
  const handler = h.handlers.get(method)
  if (!handler) throw new Error(`no handler ${method}`)
  return handler(params as never)
}

let teardown: () => void
let root: string

beforeAll(() => registerArtifactMethods(() => 50191))

beforeEach(() => {
  vi.useFakeTimers()
  teardown = initTestDatabase()
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-artifact-methods-'))
  h.sessions.length = 0
  h.writes.length = 0
  h.broadcasts.length = 0
  h.bridge.mockReset()
  h.bridge.mockResolvedValue({})
  h.sessions.push({
    id: 's1',
    projectName: 'triage',
    projectPath: root,
    status: 'running',
    statusSource: 'hooks'
  })
})

afterEach(() => {
  vi.useRealTimers()
  teardown()
  fs.rmSync(root, { recursive: true, force: true })
})

const publish = (extra: Record<string, unknown> = {}) =>
  call('artifact:publish', {
    sessionId: 's1',
    kind: 'page',
    title: 'Figures',
    content: '<p>Opus 2/9 on this fault.</p>',
    ...extra
  })

const quote = { kind: 'quote', quote: 'Opus 2/9', prefix: '', suffix: ' on' }

describe('artifact methods', () => {
  it('publishes, opens the pane and announces the version', async () => {
    const out = await publish()
    expect(out.url).toMatch(/^http:\/\/127\.0\.0\.1:50191\/artifact\//)
    expect(out.opened).toBe(true)
    expect(h.bridge).toHaveBeenCalledWith(
      'browser:openPane',
      expect.objectContaining({
        sessionId: 's1',
        artifact: expect.objectContaining({ version: 1, kind: 'page', title: 'Figures' })
      })
    )
    expect(h.broadcasts.map((b) => b.method)).toContain(IPC.ARTIFACT_PUBLISHED)
  })

  it('still publishes when the pane cannot open, and skips it when asked', async () => {
    h.bridge.mockRejectedValueOnce(new Error('no window'))
    expect((await publish()).opened).toBe(false)
    const quiet = await publish({ open: false })
    expect(quiet.opened).toBe(false)
    expect(h.bridge).toHaveBeenCalledTimes(1)
  })

  it('refuses a session that is not running', () => {
    expect(() => call('artifact:list', { sessionId: 'gone' })).toThrow('Session not found')
  })

  it('lists by session or by project, and gets one with its versions', async () => {
    const { artifact } = await publish()
    expect(call('artifact:list', { sessionId: 's1' }).map((a: { id: string }) => a.id)).toEqual([
      artifact.id
    ])
    expect(call('artifact:list', { projectName: 'triage' })).toHaveLength(1)
    const got = call('artifact:get', { artifactId: artifact.id })
    expect(got.versions).toHaveLength(1)
    expect(got.queued).toBe(false)
    expect(call('artifact:get', { artifactId: 'nope' })).toBeNull()
  })

  it('gives a version url only for versions that exist', async () => {
    const { artifact } = await publish()
    expect(call('artifact:versionUrl', { artifactId: artifact.id }).url).toContain(
      `/artifact/${artifact.id}/1?t=`
    )
    expect(call('artifact:versionUrl', { artifactId: artifact.id, version: 2 })).toBeNull()
    expect(call('artifact:versionUrl', { artifactId: 'nope' })).toBeNull()
  })

  it('saves, edits and deletes draft comments, announcing each change', async () => {
    const { artifact } = await publish()
    const c = call('artifact:saveComment', {
      artifactId: artifact.id,
      version: 1,
      anchor: quote,
      body: 'Say why'
    })
    expect(c.state).toBe('draft')
    expect(call('artifact:updateComment', { commentId: c.id, body: 'Say why last' }).body).toBe(
      'Say why last'
    )
    expect(
      call('artifact:readComments', { sessionId: 's1', artifactId: artifact.id })
    ).toHaveLength(1)
    expect(
      call('artifact:readComments', { sessionId: 's1', artifactId: artifact.id, version: 2 })
    ).toHaveLength(0)
    expect(call('artifact:deleteComment', { commentId: c.id })).toEqual({ deleted: true })
    expect(call('artifact:deleteComment', { commentId: c.id })).toEqual({ deleted: false })
    const changed = h.broadcasts.filter((b) => b.method === IPC.ARTIFACT_COMMENTS_CHANGED)
    expect(changed).toHaveLength(3)
  })

  it('refuses comments on a missing artifact or version', async () => {
    const { artifact } = await publish()
    expect(() =>
      call('artifact:saveComment', { artifactId: 'nope', version: 1, anchor: null, body: 'x' })
    ).toThrow('Artifact not found')
    expect(() =>
      call('artifact:saveComment', { artifactId: artifact.id, version: 3, anchor: null, body: 'x' })
    ).toThrow('has no version 3')
  })

  it('hides an artifact from a session in another project', async () => {
    const { artifact } = await publish()
    h.sessions.push({ id: 's2', projectName: 'other', projectPath: root, status: 'idle' })
    expect(() =>
      call('artifact:readComments', { sessionId: 's2', artifactId: artifact.id })
    ).toThrow('No artifact')
  })

  it('queues a send while the agent works, then pastes once it is idle', async () => {
    const { artifact } = await publish()
    call('artifact:saveComment', {
      artifactId: artifact.id,
      version: 1,
      anchor: quote,
      body: 'Why'
    })
    expect(call('artifact:send', { artifactId: artifact.id })).toEqual({
      state: 'queued',
      count: 0
    })
    expect(call('artifact:get', { artifactId: artifact.id }).queued).toBe(true)
    h.sessions[0].status = 'idle'
    h.listeners.get('client-message')!(IPC.SESSION_UPDATED, { id: 's1' })
    vi.runAllTimers()
    expect(h.writes.map((w) => w.data).join('')).toContain('Opus 2/9')
    expect(h.writes.at(-1)!.data).toBe('\r')
  })

  it('forgets a queued send when its session ends', async () => {
    const { artifact } = await publish()
    call('artifact:saveComment', { artifactId: artifact.id, version: 1, anchor: null, body: 'x' })
    call('artifact:send', { artifactId: artifact.id })
    h.listeners.get('session-exit')!({ id: 's1' })
    expect(call('artifact:get', { artifactId: artifact.id }).queued).toBe(false)
  })

  it('reads a version source and saves a user version with its edits', async () => {
    const { artifact } = await publish({
      kind: 'doc',
      title: 'Guide',
      content: '# Guide\n\nOld words.'
    })
    expect(
      call('artifact:readSource', { sessionId: 's1', artifactId: artifact.id }).body
    ).toContain('Old words.')
    const saved = call('artifact:saveUserVersion', {
      artifactId: artifact.id,
      body: '# Guide\n\nNew words.',
      edits: [{ before: 'Old words.', after: 'New words.' }],
      send: false
    })
    expect(saved).toMatchObject({ version: { version: 2, author: 'user' }, sent: null })
    expect(call('artifact:readSource', { artifactId: artifact.id, version: 2 }).body).toContain(
      'New words.'
    )
  })

  it('keeps a saved version when the send fails', async () => {
    const { artifact } = await publish({ kind: 'doc', title: 'Guide', content: '# Guide\n\nA.' })
    h.sessions.length = 0
    const saved = call('artifact:saveUserVersion', {
      artifactId: artifact.id,
      body: '# Guide\n\nB.',
      edits: [{ before: 'A.', after: 'B.' }],
      send: true
    })
    expect(saved.version.version).toBe(2)
    expect(saved.sent).toBeNull()
    expect(saved.sendError).toMatch(/ended/)
  })

  it('sends right away when the agent is idle', async () => {
    h.sessions[0].status = 'idle'
    const { artifact } = await publish({ kind: 'doc', title: 'Guide', content: '# Guide\n\nA.' })
    const saved = call('artifact:saveUserVersion', {
      artifactId: artifact.id,
      body: '# Guide\n\nB.',
      edits: [{ before: 'A.', after: 'B.' }],
      send: true
    })
    expect(saved.sent).toEqual({ state: 'delivered', count: 1 })
  })

  it('finds a gate page and seals its drafts', () => {
    expect(call('artifact:forGate', { runId: 'r1', nodeId: 'n1' })).toBeNull()
    const { artifact } = publishGateArtifact(
      getDataDir(),
      { runId: 'r1', nodeId: 'n1', title: 'Review' },
      '<p>Draft</p>'
    )
    const found = call('artifact:forGate', { runId: 'r1', nodeId: 'n1' })
    expect(found.artifact.id).toBe(artifact.id)
    call('artifact:saveComment', { artifactId: artifact.id, version: 1, anchor: null, body: 'x' })
    sealGateDrafts('r1', 'n1')
    const comments = call('artifact:get', { artifactId: artifact.id }).comments
    expect(comments[0].state).toBe('sent')
  })
})
