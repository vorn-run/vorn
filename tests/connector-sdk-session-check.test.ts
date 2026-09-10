import { describe, expect, it } from 'vitest'
import { checkConnector, defineConnector } from '../packages/connector-sdk/src/index'

const connector = defineConnector({
  id: 'acme',
  name: 'Acme',
  auth: {
    rung: 'browser',
    browser: {
      signInUrl: 'https://example.com/login',
      origins: ['https://example.com'],
      check: { url: 'https://example.com/me', identity: ['name'] }
    }
  },
  actions: [
    {
      type: 'me',
      label: 'Me',
      idempotent: true,
      run: async (_args, ctx) => ({
        status: (await ctx.session!.fetch('https://example.com/me')).status
      })
    }
  ]
})

describe('checking a connector that acts through a signed-in window', () => {
  it('serves its signed-in calls from the mock, so a conformance run reaches no real service', async () => {
    const codes = (await checkConnector(connector, { mock: true })).map((finding) => finding.code)
    expect(codes).not.toContain('mock-action-failed')
    expect(codes).not.toContain('mock-network-escape')
  })

  it('skips its signed-in calls in a live run from a terminal, which has no Vorn window', async () => {
    const codes = (await checkConnector(connector, { live: true })).map((finding) => finding.code)
    expect(codes).not.toContain('live-action-failed')
  })
})
