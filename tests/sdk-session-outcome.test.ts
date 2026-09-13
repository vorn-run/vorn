import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SdkBrowserSignIn, SessionCall, SourceConnection } from '../packages/shared/src/types'
import type { SessionGrant } from '../packages/server/src/connectors/session-bridge'
import { SdkCallError } from '../packages/server/src/connectors/native-client'

const action = vi.fn()
const grants = vi.hoisted(() => ({ current: undefined as SessionGrant | undefined }))
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartSdkClient: vi.fn(async () => ({ action })),
  sessionGrantFor: () => grants.current
}))

const bridge = vi.hoisted(() => ({ isConnected: true, request: vi.fn() }))
vi.mock('../packages/server/src/browser-bridge', () => ({ browserBridge: bridge }))

const signIns = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/database', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dbSetConnectionSignIn: signIns,
  dbSignalChange: vi.fn()
}))

import { invokeSdkAction } from '../packages/server/src/connectors/sdk'

const conn = {
  id: 'c1',
  connectorId: 'sdk',
  name: 'Substack',
  filters: { sdkConnectorId: 'substack', sdkVersion: '0.2.1' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-10T20:00:00Z'
} as SourceConnection

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
}

const refused = () =>
  new SdkCallError('action/run', -32000, 'HTTP 401', { kind: 'upstream', retryable: false })

type Answer = { calls: SessionCall[]; fails?: () => Error; output?: Record<string, unknown> }

/** The child answering each action, after making these requests through its window under the call's key. */
function child(actions: Record<string, Answer>): void {
  action.mockImplementation(async (params: { action: string; sessionCall?: string }) => {
    const { calls, fails, output } = actions[params.action]!
    if (params.sessionCall) grants.current?.calls.get(params.sessionCall)?.push(...calls)
    if (fails) throw fails()
    return output ?? {}
  })
}

const signedInThroughWindow = () => {
  grants.current = { token: 't', browser, calls: new Map() }
}

beforeEach(() => {
  action.mockReset()
  signIns.mockReset()
  bridge.request.mockReset()
  bridge.isConnected = true
  grants.current = undefined
})

describe('what a failed call of a browser connection says', () => {
  it('leaves an ordinary connection exactly as it was', async () => {
    child({ createDraft: { calls: [], fails: refused } })
    const result = await invokeSdkAction(conn, 'createDraft', {})
    expect(result).toMatchObject({ success: false, error: 'HTTP 401' })
    expect(result.errorKind).toBeUndefined()
    expect(result.sessionCalls).toBeUndefined()
    expect(action).toHaveBeenCalledWith({ action: 'createDraft', args: {} })
  })

  it('says to open Vorn when no desktop held the window', async () => {
    signedInThroughWindow()
    child({
      createDraft: {
        calls: [{ method: 'POST', path: '/api/v1/drafts', status: 'app-offline' }],
        fails: refused
      }
    })
    const result = await invokeSdkAction(conn, 'createDraft', {})
    expect(result.errorKind).toBe('app-offline')
    expect(result.error).toMatch(/Open Vorn on the desktop Substack signed in on/)
  })

  it('says it needs signing in when the site refused and the window is signed out', async () => {
    signedInThroughWindow()
    const calls: SessionCall[] = [{ method: 'POST', path: '/api/v1/drafts', status: 401 }]
    child({ createDraft: { calls, fails: refused } })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const result = await invokeSdkAction(conn, 'createDraft', {})
    expect(result.errorKind).toBe('needs-sign-in')
    expect(result.sessionCalls).toEqual(calls)
    expect(signIns).toHaveBeenCalledWith('c1', null, null)
  })

  it('keeps a refusal a plain error while the window is still signed in', async () => {
    signedInThroughWindow()
    child({
      deleteComment: {
        calls: [{ method: 'DELETE', path: '/api/v1/comment/9', status: 403 }],
        fails: refused
      }
    })
    bridge.request.mockResolvedValue({ signedIn: true, identity: 'Javier' })
    const result = await invokeSdkAction(conn, 'deleteComment', {})
    expect(result.errorKind).toBeUndefined()
    expect(result.error).toBe('HTTP 401')
    expect(signIns).not.toHaveBeenCalled()
  })

  it("never blames one step for another step's refusal on the same connection", async () => {
    signedInThroughWindow()
    child({
      createDraft: {
        calls: [{ method: 'POST', path: '/api/v1/drafts', status: 401 }],
        fails: refused
      },
      searchPosts: {
        calls: [{ method: 'GET', path: '/api/v1/post/search', status: 200 }],
        fails: () => new SdkCallError('action/run', -32000, 'No query given', { kind: 'internal' })
      }
    })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const [draft, search] = await Promise.all([
      invokeSdkAction(conn, 'createDraft', {}),
      invokeSdkAction(conn, 'searchPosts', {})
    ])
    expect(draft.errorKind).toBe('needs-sign-in')
    expect(search.errorKind).toBeUndefined()
    expect(search.sessionCalls).toEqual([
      { method: 'GET', path: '/api/v1/post/search', status: 200 }
    ])
  })

  it('hands the child the call key, keeps its calls for the log, then lets them go', async () => {
    signedInThroughWindow()
    const calls: SessionCall[] = [{ method: 'GET', path: '/api/v1/post/search', status: 200 }]
    child({ searchPosts: { calls, output: { posts: [] } } })
    const result = await invokeSdkAction(conn, 'searchPosts', {})
    expect(result).toEqual({ success: true, output: { posts: [] }, sessionCalls: calls })
    expect(action).toHaveBeenCalledWith({
      action: 'searchPosts',
      args: {},
      sessionCall: expect.any(String)
    })
    expect(grants.current?.calls.size).toBe(0)
  })
})

describe('what an SDK action carries both ways', () => {
  it('sends typed arguments as they are, and returns lists and nulls whole', async () => {
    child({ listPosts: { calls: [], output: { items: [{ id: 1 }], next: null } } })
    const result = await invokeSdkAction(conn, 'listPosts', {
      limit: 3,
      tags: ['ai'],
      draft: false
    })
    expect(action).toHaveBeenCalledWith({
      action: 'listPosts',
      args: { limit: 3, tags: ['ai'], draft: false }
    })
    expect(result).toEqual({ success: true, output: { items: [{ id: 1 }], next: null } })
  })

  it('reads the kind a connector gave its failure', async () => {
    child({
      offline: {
        calls: [],
        fails: () =>
          new SdkCallError('action/run', -32000, 'Vorn is closed', { kind: 'app-offline' })
      },
      signedOut: {
        calls: [],
        fails: () => new SdkCallError('action/run', -32000, 'HTTP 401', { kind: 'signed-out' })
      },
      invalid: {
        calls: [],
        fails: () =>
          new SdkCallError('action/run', -32000, 'requires "title"', {
            kind: 'validation',
            field: 'title'
          })
      }
    })
    expect((await invokeSdkAction(conn, 'offline', {})).errorKind).toBe('app-offline')
    expect((await invokeSdkAction(conn, 'signedOut', {})).errorKind).toBe('needs-sign-in')
    expect(await invokeSdkAction(conn, 'invalid', {})).toEqual({
      success: false,
      error: 'requires "title"'
    })
  })
})
