import { beforeAll, describe, expect, it } from 'vitest'
import {
  MAX_SESSION_BYTES,
  allowedHeaders,
  fetchScript,
  identityFrom,
  plainUserAgent,
  staleConnectionFolders,
  type SessionAnswer
} from '../src/main/connection-session-script'

// The page runs in Chromium, which has Uint8Array#toBase64; Node 22 does not yet.
beforeAll(() => {
  const proto = Uint8Array.prototype as unknown as Record<string, unknown>
  if (typeof proto.toBase64 !== 'function') {
    Object.defineProperty(Uint8Array.prototype, 'toBase64', {
      configurable: true,
      value(this: Uint8Array) {
        return Buffer.from(this).toString('base64')
      }
    })
  }
})

/** Run the page's script with a stand-in for the page's own fetch. */
const runScript = (script: string, answer: Response): Promise<SessionAnswer> =>
  new Function('fetch', `return ${script}`)(async () => answer) as Promise<SessionAnswer>

describe("the script a connection's hidden page runs", () => {
  it('carries the call as data, so nothing a connector sends is ever run as code', () => {
    const script = fetchScript({
      url: 'https://novumai.substack.com/api/v1/drafts/1',
      method: 'put',
      headers: { 'content-type': 'application/json', cookie: 'sid=stolen', authorization: 'x' },
      body: '"); alert(1); ("'
    })
    expect(script).toContain('fetch("https://novumai.substack.com/api/v1/drafts/1", ')
    expect(script).toContain('"method":"PUT"')
    expect(script).toContain('"credentials":"include"')
    expect(script).toContain('"body":"\\"); alert(1); (\\""')
    expect(script).not.toContain('stolen')
    expect(script).not.toContain('authorization')
  })

  it('keeps the headers a call may set, such as a CSRF flag, and drops the rest', () => {
    expect(allowedHeaders({ 'X-Csrf': 'b', Cookie: 'a' })).toEqual({ 'X-Csrf': 'b' })
  })

  it('reads bytes only when the call asks, and never passes the ask on to the site', () => {
    const plain = fetchScript({ url: 'https://cdn.example.com/0_0.png', method: 'GET' })
    expect(plain).toContain('await res.text()')
    expect(plain).not.toContain('arrayBuffer')
    const bytes = fetchScript({
      url: 'https://cdn.example.com/0_0.png',
      method: 'GET',
      binaryBody: true
    })
    expect(bytes).toContain('await res.arrayBuffer()')
    expect(bytes).not.toContain('binaryBody')
  })

  it('hands back the exact bytes the site sent', async () => {
    const png = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 255])
    const script = fetchScript({
      url: 'https://cdn.example.com/0_0.png',
      method: 'GET',
      binaryBody: true
    })
    const answer = await runScript(
      script,
      new Response(png, { status: 200, headers: { 'content-type': 'image/png' } })
    )
    expect(answer).toMatchObject({
      status: 200,
      body: '',
      headers: { 'content-type': 'image/png' }
    })
    expect(Uint8Array.from(Buffer.from(answer.bodyBase64!, 'base64'))).toEqual(png)
  })

  it('refuses an answer over the byte limit rather than cutting it short', async () => {
    const script = fetchScript({
      url: 'https://cdn.example.com/big',
      method: 'GET',
      binaryBody: true
    })
    const declared = new Response('x', {
      headers: { 'content-length': String(MAX_SESSION_BYTES + 1) }
    })
    await expect(runScript(script, declared)).rejects.toThrow(/over the 16 MiB/)
    const undeclared = new Response(new Uint8Array(MAX_SESSION_BYTES + 1))
    await expect(runScript(script, undeclared)).rejects.toThrow(/over the 16 MiB/)
  })
})

describe('the user agent a connection presents', () => {
  it('drops the tokens that name Electron and Vorn, and keeps the browser underneath', () => {
    const electron =
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) vorn/0.7.0-beta.17 Chrome/140.0.7339.41 Electron/44.1.1 Safari/537.36'
    expect(plainUserAgent(electron)).toBe(
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.7339.41 Safari/537.36'
    )
  })
})

describe('who the check says is signed in', () => {
  const profile = JSON.stringify({ name: 'Javier Canizalez', handle: 'javiercanizalez', id: 7 })

  it('reads the declared fields, naming the first and the rest beside it', () => {
    expect(identityFrom(profile, ['name', 'handle'])).toBe('Javier Canizalez (javiercanizalez)')
    expect(identityFrom(profile, ['name'])).toBe('Javier Canizalez')
    expect(identityFrom(JSON.stringify({ user: { email: 'a@b.c' } }), ['user.email'])).toBe('a@b.c')
  })

  it('names no one when the answer does not say', () => {
    expect(identityFrom(profile, ['missing'])).toBeNull()
    expect(identityFrom('<html>', ['name'])).toBeNull()
  })
})

describe('which profile folders the sweep removes', () => {
  it("removes only a connection profile whose connection is gone, never a pane's", () => {
    const folders = ['vorn-connection-a', 'vorn-connection-b', 'vorn-browser-a', 'Default']
    expect(staleConnectionFolders(folders, ['a'])).toEqual(['vorn-connection-b'])
  })
})
