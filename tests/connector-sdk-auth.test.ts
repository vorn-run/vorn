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
