import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SdkBrowserSignIn, SessionCall, SourceConnection } from '../src/shared/types'

const callTool = vi.fn()
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: vi.fn(async () => ({ callTool }))
}))

const session = vi.hoisted(() => ({
  browser: undefined as SdkBrowserSignIn | undefined,
  calls: [] as SessionCall[],
  signedIn: undefined as boolean | undefined
}))
vi.mock('../packages/server/src/connectors/session-bridge', () => ({
  browserSignInFor: () => session.browser,
  sessionCallsSince: () => session.calls,
  stillSignedIn: async () => session.signedIn
}))

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

const failing = () =>
  callTool.mockResolvedValue({ isError: true, content: [{ type: 'text', text: 'HTTP 401' }] })

beforeEach(() => {
  callTool.mockReset()
  signIns.mockReset()
  session.browser = undefined
  session.calls = []
  session.signedIn = undefined
})

describe('what a failed call of a browser connection says', () => {
  it('leaves an ordinary connection exactly as it was', async () => {
    failing()
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result).toMatchObject({ success: false, error: 'HTTP 401' })
    expect(result.errorKind).toBeUndefined()
    expect(result.sessionCalls).toBeUndefined()
    expect(callTool).toHaveBeenCalledWith({ name: 'createDraft', arguments: {} })
  })

  it('says to open Vorn when no desktop held the window', async () => {
    session.browser = browser
    session.calls = [{ method: 'POST', path: '/api/v1/drafts', status: 'app-offline' }]
    failing()
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result.errorKind).toBe('app-offline')
    expect(result.error).toMatch(/Open Vorn on the desktop Substack signed in on/)
  })

  it('says it needs signing in when the site refused and the window is signed out', async () => {
    session.browser = browser
    session.calls = [{ method: 'POST', path: '/api/v1/drafts', status: 401 }]
    session.signedIn = false
    failing()
    const result = await invokeMcpTool(conn, 'createDraft', {})
    expect(result.errorKind).toBe('needs-sign-in')
    expect(result.sessionCalls).toEqual(session.calls)
    expect(signIns).toHaveBeenCalledWith('c1', null, null)
  })

  it('keeps a refusal a plain error while the window is still signed in', async () => {
    session.browser = browser
    session.calls = [{ method: 'DELETE', path: '/api/v1/comment/9', status: 403 }]
    session.signedIn = true
    failing()
    const result = await invokeMcpTool(conn, 'deleteComment', {})
    expect(result.errorKind).toBeUndefined()
    expect(result.error).toBe('HTTP 401')
    expect(signIns).not.toHaveBeenCalled()
  })

  it('gives a browser call longer to finish, and keeps its calls for the log', async () => {
    session.browser = browser
    session.calls = [{ method: 'GET', path: '/api/v1/post/search', status: 200 }]
    callTool.mockResolvedValue({ isError: false, structuredContent: { posts: [] } })
    const result = await invokeMcpTool(conn, 'searchPosts', {})
    expect(result).toMatchObject({ success: true, sessionCalls: session.calls })
    expect(callTool).toHaveBeenCalledWith({ name: 'searchPosts', arguments: {} }, undefined, {
      timeout: 120_000
    })
  })
})
