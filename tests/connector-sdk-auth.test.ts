import { describe, expect, it } from 'vitest'
import { connectorManifest, defineConnector } from '../packages/connector-sdk/src/index'
import type { ConnectorAuth, ConnectorConfigField } from '../packages/connector-sdk/src/types'

/** A connector that is valid apart from whatever the case under test changes. */
const withAuth = (auth: ConnectorAuth, config: ConnectorConfigField[] = []) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    auth,
    config,
    actions: [{ type: 'ping', label: 'Ping', run: () => ({ ok: true }) }]
  })

describe('a declared auth rung', () => {
  it('is optional, because a connector built before rungs existed still loads', () => {
    const connector = defineConnector({
      id: 'acme',
      name: 'Acme',
      actions: [{ type: 'ping', label: 'Ping', run: () => ({}) }]
    })
    expect(connector.auth).toBeUndefined()
    expect(connectorManifest(connector).auth).toBeUndefined()
  })

  it('refuses a rung this build cannot act on', () => {
    expect(() => withAuth({ rung: 'sso' as never })).toThrow(/unknown auth rung/)
  })

  it('reaches the manifest whole, so the app can say how it signs in', () => {
    const auth: ConnectorAuth = {
      rung: 'cli',
      probe: { command: 'glab', args: ['auth', 'status'] },
      borrow: { tokenArgs: ['auth', 'token'] }
    }
    expect(connectorManifest(withAuth(auth)).auth).toEqual(auth)
  })
})

describe('what each rung has to back up', () => {
  it('makes a borrowed login name the command that asks who you are', () => {
    expect(() => withAuth({ rung: 'cli' })).toThrow(/no probe command/)
    expect(() => withAuth({ rung: 'cli', probe: { command: '  ' } })).toThrow(/no probe command/)
    expect(() => withAuth({ rung: 'cli', probe: { command: 'glab' } })).not.toThrow()
  })

  it('makes a key name the field that holds it', () => {
    const field: ConnectorConfigField = { key: 'apiToken', label: 'API token', secret: true }
    expect(() => withAuth({ rung: 'key' }, [field])).toThrow(/names no config field/)
    expect(() => withAuth({ rung: 'key', keys: ['nope'] }, [field])).toThrow(
      /names auth key "nope"/
    )
    expect(() => withAuth({ rung: 'key', keys: ['apiToken'] }, [field])).not.toThrow()
  })

  it('holds "no sign-in" to its word', () => {
    const secret: ConnectorConfigField = { key: 'apiToken', label: 'API token', secret: true }
    const plain: ConnectorConfigField = { key: 'baseUrl', label: 'Base URL' }
    expect(() => withAuth({ rung: 'none' }, [secret])).toThrow(/secret field "apiToken"/)
    expect(() => withAuth({ rung: 'none' }, [plain])).not.toThrow()
  })

  it('leaves oauth alone, since no host carries it yet', () => {
    expect(() => withAuth({ rung: 'oauth' })).not.toThrow()
  })
})

describe('a connector that signs in through a Vorn window', () => {
  const browser = {
    signInUrl: 'https://substack.com/sign-in',
    origins: ['https://substack.com', 'https://*.substack.com'],
    check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name', 'handle'] }
  }

  it('reaches the manifest whole, so the app knows where to sign in and what to check', () => {
    expect(connectorManifest(withAuth({ rung: 'browser', browser })).auth).toEqual({
      rung: 'browser',
      browser
    })
  })

  it('has to say where it signs in', () => {
    expect(() => withAuth({ rung: 'browser' })).toThrow(/declares no browser sign-in/)
  })

  it('names its origins as https hosts', () => {
    expect(() => withAuth({ rung: 'browser', browser: { ...browser, origins: [] } })).toThrow(
      /must name its origins/
    )
    expect(() =>
      withAuth({ rung: 'browser', browser: { ...browser, origins: ['http://substack.com'] } })
    ).toThrow(/"http:\/\/substack\.com" is neither/)
  })

  it('keeps its sign-in page and its check inside those origins', () => {
    expect(() =>
      withAuth({ rung: 'browser', browser: { ...browser, signInUrl: 'https://evil.io/login' } })
    ).toThrow(/sign-in page .* outside its origins/)
    expect(() =>
      withAuth({
        rung: 'browser',
        browser: { ...browser, check: { ...browser.check, url: 'https://evil.io/me' } }
      })
    ).toThrow(/signed-in check .* outside its origins/)
  })

  it('names the fields that say who is signed in as words', () => {
    expect(() =>
      withAuth({
        rung: 'browser',
        browser: { ...browser, check: { ...browser.check, identity: [' '] } }
      })
    ).toThrow(/identity fields/)
  })

  it('carries headers for its check, for a site whose reads need a CSRF header', () => {
    const check = { ...browser.check, headers: { 'X-CSRF-Protection': '1' } }
    expect(
      connectorManifest(withAuth({ rung: 'browser', browser: { ...browser, check } })).auth
    ).toEqual({
      rung: 'browser',
      browser: { ...browser, check }
    })
  })

  it('refuses a check header the browser sets itself or that says who you are', () => {
    const cookie = { ...browser.check, headers: { Cookie: '1' } }
    expect(() => withAuth({ rung: 'browser', browser: { ...browser, check: cookie } })).toThrow(
      /"Cookie" is not one/
    )
    const notText = { ...browser.check, headers: { 'X-Count': 2 as unknown as string } }
    expect(() => withAuth({ rung: 'browser', browser: { ...browser, check: notText } })).toThrow(
      /"X-Count" is not one/
    )
  })

  it('keeps no secret of its own, since the signed-in window is the secret', () => {
    const secret: ConnectorConfigField = { key: 'apiToken', label: 'API token', secret: true }
    expect(() => withAuth({ rung: 'browser', browser }, [secret])).toThrow(
      /signs in through a Vorn window but declares secret field "apiToken"/
    )
  })
})
