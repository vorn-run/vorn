import { describe, expect, it, vi } from 'vitest'
import {
  createConnectorHarness,
  defineConnector,
  runAction,
  runOptions,
  runPoll
} from '../packages/connector-sdk/src/index'
import type { ConnectorAuth, SessionContext } from '../packages/connector-sdk/src/types'

const signedIn: ConnectorAuth = {
  rung: 'browser',
  browser: {
    signInUrl: 'https://example.com/login',
    origins: ['https://example.com'],
    check: { url: 'https://example.com/me', identity: ['name'] }
  }
}

const answer = (from: string) =>
  vi.fn<typeof fetch>(
    async () =>
      new Response(JSON.stringify({ from }), { headers: { 'content-type': 'application/json' } })
  )

function acme(auth?: ConnectorAuth) {
  const seen: Array<SessionContext | undefined> = []
  const connector = defineConnector({
    id: 'acme',
    name: 'Acme',
    ...(auth && { auth }),
    options: {
      things: (ctx) => {
        seen.push(ctx.session)
        return ['one']
      }
    },
    triggers: [
      {
        type: 'changed',
        label: 'Changed',
        description: 'Something changed.',
        dedupe: 'lastItem',
        fetch: (ctx) => {
          seen.push(ctx.session)
          return []
        }
      }
    ],
    actions: [
      {
        type: 'peek',
        label: 'Peek',
        run: (_args, ctx) => {
          seen.push(ctx.session)
          return {}
        }
      },
      { type: 'me', label: 'Me', idempotent: true, request: { url: 'https://example.com/me' } }
    ]
  })
  return { connector, seen }
}

describe('the signed-in window a connector is handed', () => {
  it('goes only to a connector that signs in through one', async () => {
    const plain = acme()
    await runAction(plain.connector, 'peek', {}, { sessionFetchImpl: answer('window') })
    expect(plain.seen).toEqual([undefined])

    const browser = acme(signedIn)
    await runAction(browser.connector, 'peek', {}, { sessionFetchImpl: answer('window') })
    expect(browser.seen[0]?.fetch).toBeTypeOf('function')
  })

  it("carries a browser connector's declared calls, and leaves another connector's alone", async () => {
    const plain = answer('plain')
    const window = answer('window')
    await runAction(
      acme(signedIn).connector,
      'me',
      {},
      { fetchImpl: plain, sessionFetchImpl: window }
    )
    expect(window).toHaveBeenCalledOnce()
    expect(plain).not.toHaveBeenCalled()

    const plainAgain = answer('plain')
    const windowAgain = answer('window')
    await runAction(
      acme().connector,
      'me',
      {},
      { fetchImpl: plainAgain, sessionFetchImpl: windowAgain }
    )
    expect(plainAgain).toHaveBeenCalledOnce()
    expect(windowAgain).not.toHaveBeenCalled()
  })

  it('reaches polls and option lists too, so a trigger can read a signed-in page', async () => {
    const { connector, seen } = acme(signedIn)
    await runOptions(connector, 'things', { sessionFetchImpl: answer('window') })
    await runPoll(connector, 'changed', { sessionFetchImpl: answer('window') })
    expect(seen).toHaveLength(2)
    expect(seen.every((session) => typeof session?.fetch === 'function')).toBe(true)
  })

  it('lets one harness stub answer both kinds of call', async () => {
    const stub = answer('stub')
    await createConnectorHarness(acme(signedIn).connector, { fetchImpl: stub }).execute('me')
    expect(stub).toHaveBeenCalledOnce()
  })
})
