import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SdkBrowserSignIn, SessionCall, SourceConnection } from '../packages/shared/src/types'
import type { SessionGrant } from '../packages/server/src/connectors/session-bridge'

const { poll, getOrStartSdkClient, grants } = vi.hoisted(() => {
  const poll = vi.fn()
  return {
    poll,
    getOrStartSdkClient: vi.fn(async () => ({ poll })),
    grants: { current: undefined as SessionGrant | undefined }
  }
})
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartSdkClient,
  sessionGrantFor: () => grants.current
}))

const bridge = vi.hoisted(() => ({ isConnected: true, request: vi.fn() }))
vi.mock('../packages/server/src/browser-bridge', () => ({ browserBridge: bridge }))
vi.mock('../packages/server/src/database', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dbSetConnectionSignIn: vi.fn(),
  dbSignalChange: vi.fn()
}))

import { pollSdkConnection } from '../packages/server/src/connectors/sdk'
import {
  SdkDetectionError,
  outdatedConnectorMessage
} from '../packages/server/src/connectors/sdk-client'

const conn = {
  id: 'c1',
  connectorId: 'sdk',
  name: 'Substack',
  filters: { sdkConnectorId: 'substack', sdkVersion: '0.2.1', sdkTrigger: 'newPost' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-10T20:00:00Z'
} as SourceConnection

const item = (id: string, extra: Record<string, unknown> = {}) => ({
  externalId: id,
  title: `Post ${id}`,
  url: `https://example.substack.com/p/${id}`,
  description: '',
  status: 'published',
  labels: [],
  updatedAt: '2026-09-12T10:00:00.000Z',
  ...extra
})

beforeEach(() => {
  poll.mockReset()
  getOrStartSdkClient.mockReset().mockImplementation(async () => ({ poll }))
  bridge.request.mockReset()
  grants.current = undefined
})

describe('pollSdkConnection', () => {
  it('turns each item into an event, keeping everything the connector sent', async () => {
    poll.mockResolvedValue({
      items: [item('1', { author: 'N' })],
      nextCursor: 'c2',
      hasMore: false
    })
    const result = await pollSdkConnection(conn, 'newPost')
    expect(result).toEqual({
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
    poll.mockResolvedValue({ items: [], hasMore: false })
    await pollSdkConnection(conn, 'newPost', 'c1')
    expect(poll).toHaveBeenCalledWith({ trigger: 'newPost', cursor: 'c1' })
  })

  it('passes on that there is more, and leaves the cursor alone when none came back', async () => {
    poll.mockResolvedValue({ items: [item('1')], hasMore: true, nextCursor: 'c9' })
    expect(await pollSdkConnection(conn, 'newPost')).toMatchObject({
      nextCursor: 'c9',
      hasMore: true
    })
    poll.mockResolvedValue({ items: [], hasMore: false })
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
    const browser: SdkBrowserSignIn = {
      signInUrl: 'https://substack.com/sign-in',
      origins: ['https://substack.com'],
      check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
    }
    grants.current = { token: 't', browser, calls: new Map() }
    const calls: SessionCall[] = [{ method: 'GET', path: '/api/v1/archive', status: 401 }]
    poll.mockImplementation(async (params: { sessionCall?: string }) => {
      if (params.sessionCall) grants.current?.calls.get(params.sessionCall)?.push(...calls)
      throw new Error('HTTP 401')
    })
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    await expect(pollSdkConnection(conn, 'newPost')).rejects.toThrow(
      'Substack was signed out. Sign in again'
    )
    expect(poll).toHaveBeenCalledWith({ trigger: 'newPost', sessionCall: expect.any(String) })
    expect(grants.current.calls.size).toBe(0)
  })
})
