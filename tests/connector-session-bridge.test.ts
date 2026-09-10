import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Fastify, { type FastifyInstance } from 'fastify'
import type { SdkBrowserSignIn } from '@vornrun/shared/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const bridge = vi.hoisted(() => ({ isConnected: true, request: vi.fn() }))
vi.mock('../packages/server/src/browser-bridge', () => ({ browserBridge: bridge }))

const {
  registerSessionBridge,
  sessionEnvFor,
  setSessionBridgeOrigin,
  sessionCallsSince,
  forgetSessionGrant,
  stillSignedIn
} = await import('../packages/server/src/connectors/session-bridge')

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com', 'https://*.substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name'] }
}

let app: FastifyInstance
let token: string

beforeEach(async () => {
  bridge.isConnected = true
  bridge.request.mockReset()
  setSessionBridgeOrigin('http://127.0.0.1:4100')
  token = sessionEnvFor('c1', browser).VORN_BROWSER_TOKEN!
  app = Fastify()
  registerSessionBridge(app)
  await app.ready()
})

afterEach(async () => {
  forgetSessionGrant('c1')
  await app.close()
})

const call = (body: unknown, auth = `Bearer ${token}`, id = 'c1') =>
  app.inject({
    method: 'POST',
    url: `/connections/${id}/browser/fetch`,
    headers: { authorization: auth, 'content-type': 'application/json' },
    payload: JSON.stringify(body)
  })

describe('the endpoint a browser connector calls through', () => {
  it('hands each child its own address and a token of its own', () => {
    const env = sessionEnvFor('c2', browser)
    expect(env.VORN_BROWSER_HOST).toBe('http://127.0.0.1:4100/connections/c2/browser')
    expect(env.VORN_BROWSER_TOKEN).not.toBe(token)
    forgetSessionGrant('c2')
  })

  it('runs a call in the window and records what it asked and how it was answered', async () => {
    bridge.request.mockResolvedValue({ status: 201, headers: {}, body: '{"id":7}' })
    const since = Date.now()
    const res = await call({
      url: 'https://novumai.substack.com/api/v1/drafts',
      method: 'post',
      headers: { 'content-type': 'application/json' },
      body: '{}'
    })
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
    expect(sessionCallsSince('c1', since)).toEqual([
      { method: 'POST', path: '/api/v1/drafts', status: 201 }
    ])
  })

  it("refuses a caller without the token, or one holding another connection's", async () => {
    expect(
      (await call({ url: 'https://substack.com/', method: 'GET' }, 'Bearer wrong')).statusCode
    ).toBe(401)
    expect(
      (await call({ url: 'https://substack.com/', method: 'GET' }, `Bearer ${token}`, 'c9'))
        .statusCode
    ).toBe(401)
    expect(bridge.request).not.toHaveBeenCalled()
  })

  it("refuses a page outside the connection's origins, and a method it may not use", async () => {
    const elsewhere = await call({ url: 'https://evil.io/', method: 'GET' })
    expect(elsewhere.statusCode).toBe(403)
    expect(elsewhere.json().error).toMatch(/not on one of this connection's origins/)
    expect((await call({ url: 'https://substack.com/', method: 'TRACE' })).statusCode).toBe(405)
    expect(bridge.request).not.toHaveBeenCalled()
  })

  it('says to open Vorn when no desktop holds the window, and remembers that it could not', async () => {
    bridge.isConnected = false
    const since = Date.now()
    const res = await call({ url: 'https://substack.com/api/v1/user/profile/self', method: 'GET' })
    expect(res.statusCode).toBe(503)
    expect(res.json().error).toMatch(/Open Vorn on the desktop/)
    expect(sessionCallsSince('c1', since)).toEqual([
      { method: 'GET', path: '/api/v1/user/profile/self', status: 'app-offline' }
    ])
  })

  it('asks the desktop whether the window is still signed in, and says nothing when it cannot', async () => {
    bridge.request.mockResolvedValue({ signedIn: false, identity: null })
    expect(await stillSignedIn('c1')).toBe(false)
    expect(bridge.request).toHaveBeenCalledWith(
      'session:check',
      { connectionId: 'c1', browser },
      20_000
    )
    bridge.isConnected = false
    expect(await stillSignedIn('c1')).toBeUndefined()
  })
})
