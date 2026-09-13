import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineConnector, SESSION_CALL_HEADER } from '../packages/connector-sdk/src/index'
import { SESSION_CALL_HEADER as SHARED_HEADER } from '../packages/shared/src/types'
import { greeted } from './helpers/connector-server'

const archive = 'https://example.com/archive'

const connector = defineConnector({
  id: 'pub',
  name: 'Pub',
  auth: {
    rung: 'browser',
    browser: {
      signInUrl: 'https://example.com/login',
      origins: ['https://example.com'],
      check: { url: 'https://example.com/me', identity: ['name'] }
    }
  },
  triggers: [
    {
      type: 'changed',
      label: 'Changed',
      dedupe: 'lastItem',
      fetch: async (ctx) => {
        await ctx.session!.fetch(archive)
        return []
      }
    }
  ],
  actions: [
    {
      type: 'peek',
      label: 'Peek',
      run: async (_args, ctx) => ({ status: (await ctx.session!.fetch(archive)).status })
    },
    {
      type: 'whoami',
      label: 'Who am I',
      request: { method: 'GET', url: 'https://example.com/me' }
    }
  ]
})

afterEach(() => {
  vi.unstubAllEnvs()
  vi.unstubAllGlobals()
})

/** The connector after its hello, with its window endpoint answered by a stub. */
async function served(reply: () => Response = () => Response.json({ status: 200, body: '{}' })) {
  vi.stubEnv('VORN_BROWSER_HOST', 'http://127.0.0.1:4100/connections/c1/browser')
  vi.stubEnv('VORN_BROWSER_TOKEN', 't0k')
  const endpoint = vi.fn<typeof fetch>(async () => reply())
  vi.stubGlobal('fetch', endpoint)
  return { server: await greeted(connector), endpoint }
}

describe('the key Vorn gives a call', () => {
  it('rides on each request an action makes through the window', async () => {
    const { server, endpoint } = await served()
    await server.call('action/run', { action: 'peek', args: {}, sessionCall: 'k1' })
    expect(endpoint.mock.calls[0]?.[1]?.headers).toMatchObject({ [SESSION_CALL_HEADER]: 'k1' })
  })

  it('rides on a poll too, so a poll that meets a sign-out is told apart', async () => {
    const { server, endpoint } = await served()
    await server.call('trigger/poll', { trigger: 'changed', sessionCall: 'k2' })
    expect(endpoint.mock.calls[0]?.[1]?.headers).toMatchObject({ [SESSION_CALL_HEADER]: 'k2' })
  })

  it('is named the way the app reads it', () => {
    expect(SESSION_CALL_HEADER).toBe(SHARED_HEADER)
  })
})

describe('a signed-in call that fails', () => {
  it('reads as signed out when the service refuses who is signed in', async () => {
    const { server } = await served(() => Response.json({ status: 401, body: '' }))
    expect(await server.fail('action/run', { action: 'whoami', args: {} })).toMatchObject({
      code: -32000,
      data: { kind: 'signed-out', retryable: false }
    })
  })

  it('reads as Vorn being unreachable when the window cannot be asked', async () => {
    const { server } = await served(() => new Response('Vorn is closed', { status: 503 }))
    expect(await server.fail('action/run', { action: 'peek', args: {} })).toEqual({
      code: -32000,
      message: 'Vorn is closed',
      data: { kind: 'app-offline', retryable: false }
    })
  })
})
