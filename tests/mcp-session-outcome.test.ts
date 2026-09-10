import { describe, it, expect, vi, beforeEach } from 'vitest'
import { SESSION_CALL_META } from '../packages/shared/src/types'
import type { SdkBrowserSignIn, SessionCall, SourceConnection } from '../src/shared/types'
import type { SessionGrant } from '../packages/server/src/connectors/session-bridge'

const callTool = vi.fn()
const grants = vi.hoisted(() => ({ current: undefined as SessionGrant | undefined }))
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: vi.fn(async () => ({ callTool })),
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

import { invokeMcpTool } from '../packages/server/src/connectors/mcp'

const conn = {
  id: 'c1',
  connectorId: 'mcp',
  name: 'Substack',
  filters: { command: 'node', args: '[]' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-10T20:00:00Z'
} as SourceConnection

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
}

const refused = { isError: true, content: [{ type: 'text', text: 'HTTP 401' }] }

/** The child answering each tool, after making these requests through its window under the call's key. */
function child(tools: Record<string, { calls: SessionCall[]; answer: object }>): void {
  callTool.mockImplementation(async (params: { name: string; _meta?: Record<string, unknown> }) => {
    const { calls, answer } = tools[params.name]!
    const key = params._meta?.[SESSION_CALL_META]
    if (typeof key === 'string') grants.current?.calls.get(key)?.push(...calls)
    return answer
  })
}

const signedInThroughWindow = () => {
  grants.current = { token: 't', browser, calls: new Map() }
}

beforeEach(() => {
  callTool.mockReset()
  signIns.mockReset()
  bridge.request.mockReset()
  bridge.isConnected = true
  grants.current = undefined
})

describe('what a failed call of a browser connection says', () => {
  it('leaves an ordinary connection exactly as it was', async () => {
    child({ createDraft: { calls: [], answer: refused } })
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result).toMatchObject({ success: false, error: 'HTTP 401' })
    expect(result.errorKind).toBeUndefined()
    expect(result.sessionCalls).toBeUndefined()
    expect(callTool).toHaveBeenCalledWith({ name: 'createDraft', arguments: {} })
  })

  it('says to open Vorn when no desktop held the window', async () => {
    signedInThroughWindow()
    child({
      createDraft: {
        calls: [{ method: 'POST', path: '/api/v1/drafts', status: 'app-offline' }],
        answer: refused
      }
    })
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result.errorKind).toBe('app-offline')
    expect(result.error).toMatch(/Open Vorn on the desktop Substack signed in on/)
  })

  it('says it needs signing in when the site refused and the window is signed out', async () => {
    signedInThroughWindow()
    const calls: SessionCall[] = [{ method: 'POST', path: '/api/v1/drafts', status: 401 }]
    child({ createDraft: { calls, answer: refused } })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result.errorKind).toBe('needs-sign-in')
    expect(result.sessionCalls).toEqual(calls)
    expect(signIns).toHaveBeenCalledWith('c1', null, null)
  })

  it('keeps a refusal a plain error while the window is still signed in', async () => {
    signedInThroughWindow()
    child({
      deleteComment: {
        calls: [{ method: 'DELETE', path: '/api/v1/comment/9', status: 403 }],
        answer: refused
      }
    })
    bridge.request.mockResolvedValue({ signedIn: true, identity: 'Javier' })
    const result = await invokeMcpTool(conn, 'deleteComment', {})
    expect(result.errorKind).toBeUndefined()
    expect(result.error).toBe('HTTP 401')
    expect(signIns).not.toHaveBeenCalled()
  })

  it("never blames one step for another step's refusal on the same connection", async () => {
    signedInThroughWindow()
    child({
      createDraft: {
        calls: [{ method: 'POST', path: '/api/v1/drafts', status: 401 }],
        answer: refused
      },
      searchPosts: {
        calls: [{ method: 'GET', path: '/api/v1/post/search', status: 200 }],
        answer: { isError: true, content: [{ type: 'text', text: 'No query given' }] }
      }
    })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const [draft, search] = await Promise.all([
      invokeMcpTool(conn, 'createDraft', {}),
      invokeMcpTool(conn, 'searchPosts', {})
    ])
    expect(draft.errorKind).toBe('needs-sign-in')
    expect(search.errorKind).toBeUndefined()
    expect(search.sessionCalls).toEqual([
      { method: 'GET', path: '/api/v1/post/search', status: 200 }
    ])
  })

  it('gives a browser call longer to finish, keeps its calls for the log, then lets them go', async () => {
    signedInThroughWindow()
    const calls: SessionCall[] = [{ method: 'GET', path: '/api/v1/post/search', status: 200 }]
    child({ searchPosts: { calls, answer: { isError: false, structuredContent: { posts: [] } } } })
    const result = await invokeMcpTool(conn, 'searchPosts', {})
    expect(result).toMatchObject({ success: true, sessionCalls: calls })
    expect(callTool).toHaveBeenCalledWith(
      { name: 'searchPosts', arguments: {}, _meta: { [SESSION_CALL_META]: expect.any(String) } },
      undefined,
      { timeout: 120_000 }
    )
    expect(grants.current?.calls.size).toBe(0)
  })
})
