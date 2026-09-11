import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Fastify, { type FastifyInstance } from 'fastify'
import { SESSION_CALL_HEADER, type SdkBrowserSignIn } from '@vornrun/shared/types'
import type { SessionGrant } from '../packages/server/src/connectors/session-bridge'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const bridge = vi.hoisted(() => ({ isConnected: true, request: vi.fn() }))
vi.mock('../packages/server/src/browser-bridge', () => ({ browserBridge: bridge }))

const {
  registerSessionBridge,
  mintSessionGrant,
  setSessionBridgeOrigin,
  openSessionCall,
  closeSessionCall,
  stillSignedIn
} = await import('../packages/server/src/connectors/session-bridge')

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com', 'https://*.substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
}

let app: FastifyInstance
let grant: SessionGrant
let token: string

beforeEach(async () => {
  bridge.isConnected = true
  bridge.request.mockReset()
  setSessionBridgeOrigin('http://127.0.0.1:4100')
  const minted = mintSessionGrant('c1', browser)
  grant = minted.grant
  token = minted.env.VORN_BROWSER_TOKEN!
  app = Fastify()
  registerSessionBridge(app, (id) => (id === 'c1' ? grant : undefined))
  await app.ready()
})

afterEach(async () => {
  await app.close()
})

const call = (
  body: unknown,
  { auth = `Bearer ${token}`, id = 'c1', key }: { auth?: string; id?: string; key?: string } = {}
) =>
  app.inject({
    method: 'POST',
    url: `/connections/${id}/browser/fetch`,
    headers: {
      authorization: auth,
      'content-type': 'application/json',
      ...(key && { [SESSION_CALL_HEADER]: key })
    },
    payload: JSON.stringify(body)
  })

describe('the endpoint a browser connector calls through', () => {
  it('hands each child its own address and a token of its own', () => {
    const { env } = mintSessionGrant('c2', browser)
    expect(env.VORN_BROWSER_HOST).toBe('http://127.0.0.1:4100/connections/c2/browser')
    expect(env.VORN_BROWSER_TOKEN).not.toBe(token)
  })

  it('runs a call in the window and records it under the tool call it belongs to', async () => {
    bridge.request.mockResolvedValue({ status: 201, headers: {}, body: '{"id":7}' })
    const open = openSessionCall(grant)
    const res = await call(
      {
        url: 'https://novumai.substack.com/api/v1/drafts',
        method: 'post',
        headers: { 'content-type': 'application/json' },
        body: '{}'
      },
      { key: open.key }
    )
    expect(res.statusCode).toBe(200)
    expect(res.json()).toEqual({ status: 201, headers: {}, body: '{"id":7}' })
    expect(bridge.request).toHaveBeenCalledWith(
      'session:fetch',
      {
        connectionId: 'c1',
        origins: browser.origins,
        request: {
          url: 'https://novumai.substack.com/api/v1/drafts',
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: '{}'
        }
      },
      20_000
    )
    expect(closeSessionCall(open)).toEqual([
      { method: 'POST', path: '/api/v1/drafts', status: 201 }
    ])
    expect(grant.calls.size).toBe(0)
  })

  it('keeps two tool calls on one connection apart, however their requests interleave', async () => {
    bridge.request
      .mockResolvedValueOnce({ status: 200, headers: {}, body: '[]' })
      .mockResolvedValueOnce({ status: 401, headers: {}, body: '' })
    const search = openSessionCall(grant)
    const draft = openSessionCall(grant)
    await call(
      { url: 'https://substack.com/api/v1/post/search', method: 'GET' },
      { key: search.key }
    )
    await call({ url: 'https://substack.com/api/v1/drafts', method: 'POST' }, { key: draft.key })
    expect(closeSessionCall(search)).toEqual([
      { method: 'GET', path: '/api/v1/post/search', status: 200 }
    ])
    expect(closeSessionCall(draft)).toEqual([
      { method: 'POST', path: '/api/v1/drafts', status: 401 }
    ])
  })

  it('answers a request from no known tool call without keeping it', async () => {
    bridge.request.mockResolvedValue({ status: 200, headers: {}, body: '' })
    expect((await call({ url: 'https://substack.com/', method: 'GET' })).statusCode).toBe(200)
    expect(
      (await call({ url: 'https://substack.com/', method: 'GET' }, { key: 'finished' })).statusCode
    ).toBe(200)
    expect(grant.calls.size).toBe(0)
  })

  it("refuses a caller without the token, or one holding another connection's", async () => {
    const url = 'https://substack.com/'
    expect((await call({ url, method: 'GET' }, { auth: 'Bearer wrong' })).statusCode).toBe(401)
    expect((await call({ url, method: 'GET' }, { id: 'c9' })).statusCode).toBe(401)
    expect(bridge.request).not.toHaveBeenCalled()
  })

  it("refuses a page outside the connection's origins, and a method it may not use", async () => {
    const elsewhere = await call({ url: 'https://evil.io/', method: 'GET' })
    expect(elsewhere.statusCode).toBe(403)
    expect(elsewhere.body).toMatch(/not on one of this connection's origins/)
    expect((await call({ url: 'https://substack.com/', method: 'TRACE' })).statusCode).toBe(405)
    expect(bridge.request).not.toHaveBeenCalled()
  })

  it('says to open Vorn when no desktop holds the window, and remembers that it could not', async () => {
    bridge.isConnected = false
    const open = openSessionCall(grant)
    const res = await call(
      { url: 'https://substack.com/api/v1/user/profile/self', method: 'GET' },
      { key: open.key }
    )
    expect(res.statusCode).toBe(503)
    expect(res.body).toMatch(/Open Vorn on the desktop/)
    expect(closeSessionCall(open)).toEqual([
      { method: 'GET', path: '/api/v1/user/profile/self', status: 'app-offline' }
    ])
  })
})

describe('asking whether a window is still signed in', () => {
  it('asks the desktop once for calls that failed together, and says nothing when it cannot', async () => {
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    const answers = await Promise.all([stillSignedIn('c1', browser), stillSignedIn('c1', browser)])
    expect(answers).toEqual([false, false])
    expect(bridge.request).toHaveBeenCalledTimes(1)
    expect(bridge.request).toHaveBeenCalledWith(
      'session:check',
      { connectionId: 'c1', browser },
      20_000
    )
    bridge.isConnected = false
    expect(await stillSignedIn('c1', browser)).toBeUndefined()
  })
})
