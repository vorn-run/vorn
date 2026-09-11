import { describe, expect, it } from 'vitest'
import {
  AUTH_RUNG,
  connectorAuthRungs,
  type ConnectorListing
} from '../src/renderer/lib/connector-browse'

describe('how the directory names a browser sign-in', () => {
  it('offers it among the sign-in filters, before OAuth', () => {
    const listings = [{ authRung: 'oauth' }, { authRung: 'browser' }, { authRung: 'key' }]
    expect(connectorAuthRungs(listings as ConnectorListing[])).toEqual(['key', 'browser', 'oauth'])
  })

  it('does not read like OAuth, which also opens a browser', () => {
    expect(AUTH_RUNG.browser).toEqual({
      label: 'Signs in through a Vorn window',
      badge: 'browser',
      detail: 'Signs in through a Vorn window'
    })
    expect(AUTH_RUNG.oauth.label).not.toBe(AUTH_RUNG.browser.label)
  })
})
