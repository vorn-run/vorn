import { describe, it, expect, vi, beforeEach } from 'vitest'
import type {
  ExternalItem,
  SdkBrowserSignIn,
  SessionCall,
  SourceConnection
} from '../packages/shared/src/types'
import type { SessionGrant } from '../packages/server/src/connectors/session-bridge'
import { SdkCallError } from '../packages/server/src/connectors/native-client'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

// One child answers for every connection here; each test says what it answers.
const { child, getOrStartSdkClient, grants } = vi.hoisted(() => {
  const child = { action: vi.fn(), poll: vi.fn(), preflight: vi.fn() }
  return {
    child,
    getOrStartSdkClient: vi.fn(async () => child),
    grants: { current: undefined as SessionGrant | undefined }
  }
})
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartSdkClient,
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

import {
  backfillSdkConnection,
  invokeSdkAction,
  pollSdkConnection,
  preflightSdkConnection,
  sdkConnector
} from '../packages/server/src/connectors/sdk'
import {
  SdkDetectionError,
  outdatedConnectorMessage
} from '../packages/server/src/connectors/sdk-client'

function connection(filters: Record<string, unknown> = {}): SourceConnection {
  return {
    id: 'c1',
    connectorId: 'sdk',
    name: 'Substack',
    filters: { sdkConnectorId: 'substack', sdkVersion: '0.2.1', ...filters },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-10T20:00:00Z'
  } as SourceConnection
}

const conn = connection()

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
}

const signedInThroughWindow = () => {
  grants.current = { token: 't', browser, calls: new Map() }
}

const refused = () =>
  new SdkCallError('action/run', -32000, 'HTTP 401', { kind: 'upstream', retryable: false })

type Answer = { calls: SessionCall[]; fails?: () => Error; output?: Record<string, unknown> }

/** The child answering each action, after making these requests through its window under the call's key. */
function answering(actions: Record<string, Answer>): void {
  child.action.mockImplementation(async (params: { action: string; sessionCall?: string }) => {
    const { calls, fails, output } = actions[params.action]!
    if (params.sessionCall) grants.current?.calls.get(params.sessionCall)?.push(...calls)
    if (fails) throw fails()
    return output ?? {}
  })
}

const item = (id: string, extra: Record<string, unknown> = {}) => ({
  externalId: id,
  title: `Post ${id}`,
  url: `https://example.substack.com/p/${id}`,
  description: `About ${id}`,
  status: 'published',
  labels: [],
  updatedAt: '2026-09-12T10:00:00.000Z',
  ...extra
})

beforeEach(() => {
  child.action.mockReset()
  child.poll.mockReset()
  child.preflight.mockReset()
  getOrStartSdkClient.mockReset().mockImplementation(async () => child)
  signIns.mockReset()
  bridge.request.mockReset()
  bridge.isConnected = true
  grants.current = undefined
})

describe('what a failed call of a browser connection says', () => {
  it('leaves an ordinary connection exactly as it was', async () => {
    answering({ createDraft: { calls: [], fails: refused } })
    const result = await invokeSdkAction(conn, 'createDraft', {})
    expect(result).toMatchObject({ success: false, error: 'HTTP 401' })
    expect(result.errorKind).toBeUndefined()
    expect(result.sessionCalls).toBeUndefined()
    expect(child.action).toHaveBeenCalledWith({ action: 'createDraft', args: {} })
  })

  it('says to open Vorn when no desktop held the window', async () => {
    signedInThroughWindow()
    answering({
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
    answering({ createDraft: { calls, fails: refused } })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const result = await invokeSdkAction(conn, 'createDraft', {})
    expect(result.errorKind).toBe('needs-sign-in')
    expect(result.sessionCalls).toEqual(calls)
    expect(signIns).toHaveBeenCalledWith('c1', null, null)
  })

  it('keeps a refusal a plain error while the window is still signed in', async () => {
    signedInThroughWindow()
    answering({
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
    answering({
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
    answering({ searchPosts: { calls, output: { posts: [] } } })
    const result = await invokeSdkAction(conn, 'searchPosts', {})
    expect(result).toEqual({ success: true, output: { posts: [] }, sessionCalls: calls })
    expect(child.action).toHaveBeenCalledWith({
      action: 'searchPosts',
      args: {},
      sessionCall: expect.any(String)
    })
    expect(grants.current?.calls.size).toBe(0)
  })
})

describe('what an SDK action carries both ways', () => {
  it('sends typed arguments as they are, and returns lists and nulls whole', async () => {
    answering({ listPosts: { calls: [], output: { items: [{ id: 1 }], next: null } } })
    const result = await invokeSdkAction(conn, 'listPosts', {
      limit: 3,
      tags: ['ai'],
      draft: false
    })
    expect(child.action).toHaveBeenCalledWith({
      action: 'listPosts',
      args: { limit: 3, tags: ['ai'], draft: false }
    })
    expect(result).toEqual({ success: true, output: { items: [{ id: 1 }], next: null } })
  })

  it('reads the kind a connector gave its failure', async () => {
    answering({
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

describe('pollSdkConnection', () => {
  it('turns each item into an event, keeping everything the connector sent', async () => {
    child.poll.mockResolvedValue({
      items: [item('1', { author: 'N' })],
      nextCursor: 'c2',
      hasMore: false
    })
    expect(await pollSdkConnection(conn, 'newPost')).toEqual({
      events: [
        {
          id: '1',
          type: 'mcpPoll',
          timestamp: '2026-09-12T10:00:00.000Z',
          data: item('1', { author: 'N' })
        }
      ],
      nextCursor: 'c2',
      hasMore: false
    })
  })

  it('asks for the trigger it was given, from the cursor it was given', async () => {
    child.poll.mockResolvedValue({ items: [], hasMore: false })
    await pollSdkConnection(conn, 'newPost', 'c1')
    expect(child.poll).toHaveBeenCalledWith({ trigger: 'newPost', cursor: 'c1' })
  })

  it('passes on that there is more, and leaves the cursor alone when none came back', async () => {
    child.poll.mockResolvedValue({ items: [item('1')], hasMore: true, nextCursor: 'c9' })
    expect(await pollSdkConnection(conn, 'newPost')).toMatchObject({
      nextCursor: 'c9',
      hasMore: true
    })
    child.poll.mockResolvedValue({ items: [], hasMore: false })
    expect(await pollSdkConnection(conn, 'newPost', 'c9')).not.toHaveProperty('nextCursor')
  })

  it('reports a pack built for an older Vorn in words that say how to fix it', async () => {
    getOrStartSdkClient.mockRejectedValue(
      new SdkDetectionError('outdated', outdatedConnectorMessage('Substack'))
    )
    await expect(pollSdkConnection(conn, 'newPost')).rejects.toThrow(
      outdatedConnectorMessage('Substack')
    )
  })

  it('polls through the window with a call key, and says so when the window signed out', async () => {
    signedInThroughWindow()
    const calls: SessionCall[] = [{ method: 'GET', path: '/api/v1/archive', status: 401 }]
    child.poll.mockImplementation(async (params: { sessionCall?: string }) => {
      if (params.sessionCall) grants.current?.calls.get(params.sessionCall)?.push(...calls)
      throw new Error('HTTP 401')
    })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    await expect(pollSdkConnection(conn, 'newPost')).rejects.toThrow(
      'Substack was signed out. Sign in again'
    )
    expect(child.poll).toHaveBeenCalledWith({ trigger: 'newPost', sessionCall: expect.any(String) })
    expect(grants.current?.calls.size).toBe(0)
  })
})

describe('the trigger an sdk connection polls', () => {
  it("is the connection's own when its workflow fires on the generic event", async () => {
    child.poll.mockResolvedValue({ items: [], hasMore: false })
    const pager = sdkConnector.pollConnection!(connection({ sdkTrigger: 'newPost' }), 'mcpPoll')
    if (typeof pager === 'string') throw new Error(pager)
    await pager('c1')
    expect(child.poll).toHaveBeenCalledWith({ trigger: 'newPost', cursor: 'c1' })
  })

  it('is the one a template workflow names', async () => {
    child.poll.mockResolvedValue({ items: [], hasMore: false })
    const pager = sdkConnector.pollConnection!(conn, 'issueCreated')
    if (typeof pager === 'string') throw new Error(pager)
    await pager()
    expect(child.poll).toHaveBeenCalledWith({ trigger: 'issueCreated' })
  })

  it('is none on a connection made only for its actions, which says so', () => {
    expect(sdkConnector.pollConnection!(conn, 'mcpPoll')).toBe('has no trigger to poll')
  })
})

describe('backfillSdkConnection', () => {
  async function drain(from: SourceConnection): Promise<ExternalItem[]> {
    const seen: ExternalItem[] = []
    await backfillSdkConnection(from, (entry) => seen.push(entry))
    return seen
  }

  it('says a connection with no trigger has nothing to import', async () => {
    await expect(drain(conn)).rejects.toThrow(
      'Connection "Substack" has no trigger, so there is nothing to import.'
    )
    expect(child.poll).not.toHaveBeenCalled()
  })

  it('follows the connector from no cursor while it says there is more', async () => {
    child.poll
      .mockResolvedValueOnce({ items: [item('1')], nextCursor: 'p2', hasMore: true })
      .mockResolvedValueOnce({ items: [item('2')], nextCursor: 'p3', hasMore: false })
    const seen = await drain(connection({ sdkTrigger: 'newItem' }))
    expect(seen.map((entry) => entry.externalId)).toEqual(['1', '2'])
    expect(child.poll.mock.calls.map(([params]) => params)).toEqual([
      { trigger: 'newItem' },
      { trigger: 'newItem', cursor: 'p2' }
    ])
  })

  it('stops a connector that says there is more without moving its cursor', async () => {
    child.poll.mockResolvedValue({ items: [item('1')], nextCursor: 'same', hasMore: true })
    await expect(drain(connection({ sdkTrigger: 'newItem' }))).rejects.toThrow(
      'Connection "Substack" did not advance its backfill cursor'
    )
  })

  it('carries the fields backfill upserts on, and the whole item beside them', async () => {
    child.poll.mockResolvedValue({ items: [item('1')], hasMore: false })
    const [entry] = await drain(connection({ sdkTrigger: 'newItem' }))
    expect(entry).toEqual({
      externalId: '1',
      url: 'https://example.substack.com/p/1',
      title: 'Post 1',
      description: 'About 1',
      status: 'published',
      updatedAt: '2026-09-12T10:00:00.000Z',
      metadata: item('1')
    })
  })
})

describe('preflightSdkConnection', () => {
  // "Nothing to check" and "checked, fine" are different answers; only one reads as reassurance.
  it('reports null when the connector declares no preflight', async () => {
    child.preflight.mockResolvedValue({ ok: null })
    expect(await preflightSdkConnection(conn)).toEqual({ ok: null })
  })

  it('carries the message, which is the part a user can act on', async () => {
    child.preflight.mockResolvedValue({ ok: false, message: 'Run `gh auth login`.' })
    expect(await preflightSdkConnection(conn)).toEqual({
      ok: false,
      message: 'Run `gh auth login`.'
    })
  })

  // The RPC turns a throw into a failed check, never into "nothing to check".
  it('rejects when the connector will not launch, saying how to fix a pack built for an older Vorn', async () => {
    getOrStartSdkClient.mockRejectedValue(new Error('spawn npx ENOENT'))
    await expect(preflightSdkConnection(conn)).rejects.toThrow('spawn npx ENOENT')
    getOrStartSdkClient.mockRejectedValue(
      new SdkDetectionError('outdated', outdatedConnectorMessage('Substack'))
    )
    await expect(preflightSdkConnection(conn)).rejects.toThrow(
      'Substack was built for an older Vorn. Update it in Settings → Connectors'
    )
  })
})
