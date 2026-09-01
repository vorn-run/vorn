import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  httpConnector,
  httpProfileError,
  lockedProfileError,
  performHttpRequest
} from '../packages/server/src/connectors/http'

type FetchCall = { url: URL; init: RequestInit }

function stubFetch(response: { status?: number; body?: string; contentType?: string } = {}) {
  const calls: FetchCall[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: URL, init: RequestInit) => {
      calls.push({ url, init })
      return new Response(response.body ?? '{"ok":true}', {
        status: response.status ?? 200,
        headers: { 'content-type': response.contentType ?? 'application/json' }
      })
    })
  )
  return calls
}

afterEach(() => vi.unstubAllGlobals())

describe('performHttpRequest', () => {
  it('injects the secret into the profile header and never echoes it back', async () => {
    const calls = stubFetch()
    const result = await performHttpRequest(
      {
        baseUrl: 'https://api.example.com',
        authHeader: 'Authorization: Bearer {{secret}}',
        secret: 'top-secret'
      },
      { method: 'GET', url: 'https://api.example.com/me' }
    )
    const sent = calls[0].init.headers as Record<string, string>
    expect(sent.Authorization).toBe('Bearer top-secret')
    expect(result.success).toBe(true)
    expect(JSON.stringify(result)).not.toContain('top-secret')
  })

  it('injects the secret as a query parameter', async () => {
    const calls = stubFetch()
    await performHttpRequest(
      { baseUrl: 'https://api.example.com', authQuery: 'api_key={{secret}}', secret: 's3' },
      { method: 'GET', url: 'https://api.example.com/items?limit=2' }
    )
    expect(calls[0].url.searchParams.get('api_key')).toBe('s3')
    expect(calls[0].url.searchParams.get('limit')).toBe('2')
  })

  it('merges the profile JSON into a JSON object body', async () => {
    const calls = stubFetch()
    await performHttpRequest(
      { baseUrl: 'https://api.example.com', authBody: '{"token": "{{secret}}"}', secret: 's4' },
      { method: 'POST', url: 'https://api.example.com/x', body: '{"name": "a"}' }
    )
    expect(JSON.parse(calls[0].init.body as string)).toEqual({ name: 'a', token: 's4' })
  })

  it('leaves a non-JSON body untouched by body injection', async () => {
    const calls = stubFetch()
    await performHttpRequest(
      { baseUrl: 'https://api.example.com', authBody: '{"token": "{{secret}}"}', secret: 's5' },
      { method: 'POST', url: 'https://api.example.com/x', body: 'plain text' }
    )
    expect(calls[0].init.body).toBe('plain text')
  })

  it('resolves a relative path against the profile base URL', async () => {
    const calls = stubFetch()
    await performHttpRequest(
      { baseUrl: 'https://api.example.com' },
      { method: 'GET', url: '/v1/items' }
    )
    expect(calls[0].url.toString()).toBe('https://api.example.com/v1/items')
  })

  it('returns status, headers, and a JSON-parsed body', async () => {
    stubFetch({ status: 201, body: '{"id": 7}' })
    const result = await performHttpRequest({}, { method: 'POST', url: 'https://x.test/a' })
    expect(result.output).toMatchObject({ status: 201, body: { id: 7 } })
  })

  it('keeps a non-JSON response body as text', async () => {
    stubFetch({ body: 'hello', contentType: 'text/plain' })
    const result = await performHttpRequest({}, { method: 'GET', url: 'https://x.test/a' })
    expect(result.output?.body).toBe('hello')
  })

  it('sends no body on GET', async () => {
    const calls = stubFetch()
    await performHttpRequest({}, { method: 'GET', url: 'https://x.test/a', body: 'nope' })
    expect(calls[0].init.body).toBeUndefined()
  })

  it('refuses to sign a request whose origin is not the profile base URL', async () => {
    const calls = stubFetch()
    const result = await performHttpRequest(
      {
        baseUrl: 'https://api.example.com',
        authHeader: 'Authorization: Bearer {{secret}}',
        secret: 's'
      },
      { method: 'POST', url: 'https://evil.example.net/collect' }
    )
    expect(result.success).toBe(false)
    expect(result.error).toContain('only signs requests to https://api.example.com')
    expect(calls).toHaveLength(0)
  })

  it('refuses injection entirely when the profile has no base URL', async () => {
    const calls = stubFetch()
    const result = await performHttpRequest(
      { authHeader: 'X-Key: {{secret}}', secret: 's' },
      { method: 'GET', url: 'https://anywhere.example.com/a' }
    )
    expect(result.success).toBe(false)
    expect(calls).toHaveLength(0)
  })

  it('rejects a missing or invalid method as a result, not a throw', async () => {
    const calls = stubFetch()
    const missing = await performHttpRequest({}, { method: '', url: 'https://x.test/a' })
    const bogus = await performHttpRequest({}, { method: 'YEET', url: 'https://x.test/a' })
    expect(missing).toMatchObject({ success: false })
    expect(bogus).toMatchObject({ success: false })
    expect(calls).toHaveLength(0)
  })

  it('never follows redirects', async () => {
    const calls = stubFetch()
    await performHttpRequest({}, { method: 'GET', url: 'https://x.test/a' })
    expect(calls[0].init.redirect).toBe('manual')
  })

  it('reports an invalid URL without calling out', async () => {
    const calls = stubFetch()
    const result = await performHttpRequest({}, { method: 'GET', url: 'not a url' })
    expect(result.success).toBe(false)
    expect(calls).toHaveLength(0)
  })

  it('reports a network failure as an error result', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('connect ECONNREFUSED')))
    const result = await performHttpRequest({}, { method: 'GET', url: 'https://x.test/a' })
    expect(result).toMatchObject({ success: false, error: 'connect ECONNREFUSED' })
  })
})

describe('lockedProfileError', () => {
  it('flags a stored secret with no decrypted counterpart', () => {
    expect(lockedProfileError({ secret: 'AAECbase64blob' }, undefined)).toContain('locked')
  })

  it('passes once the decrypted store holds the secret', () => {
    expect(lockedProfileError({ secret: 'blob' }, { secret: 'plain' })).toBeNull()
  })

  it('passes for a profile with no secret at all', () => {
    expect(lockedProfileError({ baseUrl: 'https://x.test' }, undefined)).toBeNull()
  })
})

/**
 * A profile id travels from the renderer as a plain string, so the request path
 * has to say what a profile is rather than trust the caller to send one.
 */
describe('httpProfileError', () => {
  it('refuses a connection belonging to another connector', () => {
    const problem = httpProfileError({ connectorId: 'github', filters: {} }, undefined)
    expect(problem).toContain('github')
    expect(problem).toContain('not an HTTP auth profile')
  })

  it('still reports a locked secret on a real profile', () => {
    const profile = { connectorId: 'http', filters: { secret: 'blob' } }
    expect(httpProfileError(profile, undefined)).toContain('locked')
  })

  it('passes an unlocked profile', () => {
    expect(
      httpProfileError({ connectorId: 'http', filters: { secret: 'blob' } }, { secret: 'plain' })
    ).toBeNull()
  })
})

describe('the http connector', () => {
  it('tests a profile with a real request against its base URL', async () => {
    const calls = stubFetch({ status: 200 })
    const result = await httpConnector.execute!('test', {
      baseUrl: 'https://api.example.com',
      authHeader: 'X-Key: {{secret}}',
      secret: 'k'
    })
    expect(result.success).toBe(true)
    expect(result.output?.status).toBe(200)
    expect((calls[0].init.headers as Record<string, string>)['X-Key']).toBe('k')
  })

  it('refuses to test a profile with no base URL', async () => {
    const calls = stubFetch()
    const result = await httpConnector.execute!('test', {})
    expect(result.success).toBe(false)
    expect(calls).toHaveLength(0)
  })

  it('executes the request action with merged profile fields', async () => {
    const calls = stubFetch()
    const result = await httpConnector.execute!('request', {
      baseUrl: 'https://api.example.com',
      authQuery: 'key={{secret}}',
      secret: 's',
      method: 'GET',
      url: '/v1/things'
    })
    expect(result.success).toBe(true)
    expect(calls[0].url.toString()).toBe('https://api.example.com/v1/things?key=s')
  })

  it('declares auth fields including an encrypted secret', () => {
    const manifest = httpConnector.describe()
    const secret = manifest.auth.find((f) => f.key === 'secret')
    expect(secret?.type).toBe('password')
    expect(manifest.auth.find((f) => f.key === 'profileName')?.required).toBe(true)
  })
})
