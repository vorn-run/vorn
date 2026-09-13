import { describe, expect, it } from 'vitest'
import {
  allowedHeaders,
  fetchScript,
  identityFrom,
  plainUserAgent,
  staleConnectionFolders
} from '../src/main/connection-session-script'

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
