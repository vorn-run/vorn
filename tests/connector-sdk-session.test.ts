import { describe, expect, it, vi } from 'vitest'
import {
  createSessionFetch,
  SessionUnavailableError,
  withinOrigins
} from '../packages/connector-sdk/src/index'
import { withinOrigins as sharedWithinOrigins } from '../packages/shared/src/connector-origins'

const env = {
  VORN_SESSION_HOST: 'http://127.0.0.1:4100/connections/c1/session',
  VORN_SESSION_TOKEN: 't0k'
}

const answering = (status: number, body: unknown) =>
  vi.fn<typeof fetch>(async () => new Response(JSON.stringify(body), { status }))

describe('the signed-in fetch a browser connector gets', () => {
  it('asks Vorn to make the call, carrying the request whole', async () => {
    const call = answering(200, {
      status: 201,
      headers: { 'content-type': 'application/json' },
      body: '{"id":7}'
    })
    const fetch = createSessionFetch({ env, fetchImpl: call })
    const res = await fetch('https://novumai.substack.com/api/v1/drafts', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{"draft_title":"t"}'
    })

    expect(res.status).toBe(201)
    expect(await res.json()).toEqual({ id: 7 })
    const [url, init] = call.mock.calls[0]!
    expect(url).toBe('http://127.0.0.1:4100/connections/c1/session/fetch')
    expect(init?.headers).toMatchObject({ authorization: 'Bearer t0k' })
    expect(JSON.parse(init?.body as string)).toEqual({
      url: 'https://novumai.substack.com/api/v1/drafts',
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{"draft_title":"t"}'
    })
  })

  it('says it needs Vorn when Vorn did not start it', async () => {
    const fetch = createSessionFetch({ env: {}, fetchImpl: answering(200, {}) })
    await expect(fetch('https://substack.com/')).rejects.toBeInstanceOf(SessionUnavailableError)
  })

  it('never sends its token to a host that is not this machine', async () => {
    const call = answering(200, {})
    const fetch = createSessionFetch({
      env: { ...env, VORN_SESSION_HOST: 'http://collector.example/session' },
      fetchImpl: call
    })
    await expect(fetch('https://substack.com/')).rejects.toThrow(/served on this machine/)
    expect(call).not.toHaveBeenCalled()
  })

  it('reads a closed app as unavailable, and any other refusal as an error', async () => {
    const closed = createSessionFetch({
      env,
      fetchImpl: answering(503, { error: 'Open Vorn on the desktop this connection signed in on' })
    })
    await expect(closed('https://substack.com/')).rejects.toThrow(SessionUnavailableError)
    await expect(closed('https://substack.com/')).rejects.toThrow(/Open Vorn on the desktop/)

    const refused = createSessionFetch({
      env,
      fetchImpl: answering(403, { error: 'https://evil.io is not one of its origins' })
    })
    const error = await refused('https://evil.io/').catch((e: unknown) => e)
    expect(error).toBeInstanceOf(Error)
    expect(error).not.toBeInstanceOf(SessionUnavailableError)
    expect((error as Error).message).toMatch(/not one of its origins/)
  })

  it('hands back a no-content answer with no body', async () => {
    const fetch = createSessionFetch({ env, fetchImpl: answering(200, { status: 204 }) })
    const res = await fetch('https://substack.com/api/v1/comment/1', { method: 'DELETE' })
    expect(res.status).toBe(204)
    expect(await res.text()).toBe('')
  })
})

describe('which pages a browser connector may reach', () => {
  const origins = ['https://substack.com', 'https://*.substack.com']
  const cases: Array<[string, boolean]> = [
    ['https://substack.com/api/v1/user/profile/self', true],
    ['https://novumai.substack.com/feed', true],
    ['https://SUBSTACK.com/sign-in', true],
    ['https://substack.com.evil.io/', false],
    ['https://evilsubstack.com/', false],
    ['http://substack.com/', false],
    ['https://substack.com:8443/', false],
    ['not a url', false]
  ]

  it('takes the named host and every subdomain of a wildcard, and nothing that only looks like them', () => {
    for (const [url, allowed] of cases)
      expect([url, withinOrigins(origins, url)]).toEqual([url, allowed])
    expect(withinOrigins(['https://*.substack.com'], 'https://substack.com/')).toBe(false)
  })

  it('draws the same line in the app as in the SDK', () => {
    for (const [url] of cases) {
      expect([url, sharedWithinOrigins(origins, url)]).toEqual([url, withinOrigins(origins, url)])
    }
  })
})
