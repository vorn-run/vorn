import { describe, expect, it } from 'vitest'
import {
  checkConnector,
  createConnectorHarness,
  defineConnector,
  withMockHttp
} from '../packages/connector-sdk/src/index'
import { mockConfig } from '../packages/connector-sdk/src/check'
import type { ActionDefinition } from '../packages/connector-sdk/src/types'

/** An action that talks to a service, the way a real connector's would. */
const post: ActionDefinition = {
  type: 'post',
  label: 'Post message',
  description: 'Send a message',
  idempotent: false,
  inputs: [{ key: 'text', label: 'Text', description: 'What to send', required: true }],
  outputs: [{ key: 'id', type: 'string' }],
  async run(args) {
    const response = await fetch('https://acme.test/api/messages', {
      method: 'POST',
      body: JSON.stringify({ text: args.text })
    })
    const body = (await response.json()) as { id?: string }
    return { id: body.id ?? '' }
  }
}

const connector = (actions: ActionDefinition[] = [post]) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    description: 'Talks to Acme',
    auth: { rung: 'none' },
    actions
  })

describe('serving a connector its HTTP in-process', () => {
  it('answers from the routes and hands back what was asked', async () => {
    const harness = createConnectorHarness(connector())
    const { result, calls } = await harness.withMockHttp(
      [{ url: '/api/messages', method: 'POST', body: { id: 'm-1' } }],
      () => harness.execute('post', { text: 'hi' })
    )

    expect(result).toEqual({ id: 'm-1' })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: 'POST', url: 'https://acme.test/api/messages' })
    expect(calls[0].body).toContain('hi')
  })

  it('refuses a call no route offered, rather than letting it reach a service', async () => {
    const harness = createConnectorHarness(connector())
    await expect(
      harness.withMockHttp([{ url: '/api/other' }], () => harness.execute('post', { text: 'hi' }))
    ).rejects.toThrow(/No mock route for POST https:\/\/acme.test\/api\/messages/)
  })

  it('serves the status a route names, so error handling can be exercised', async () => {
    const failing: ActionDefinition = {
      ...post,
      async run() {
        const response = await fetch('https://acme.test/api/messages', { method: 'POST' })
        if (!response.ok) throw new Error(`Acme said ${response.status}`)
        return {}
      }
    }
    const harness = createConnectorHarness(connector([failing]))
    await expect(
      harness.withMockHttp([{ url: '/api/', status: 503 }], () =>
        harness.execute('post', { text: 'hi' })
      )
    ).rejects.toThrow('Acme said 503')
  })

  it('gives the real fetch back, even when the body threw', async () => {
    const original = globalThis.fetch
    await expect(
      withMockHttp([], () => {
        throw new Error('boom')
      })
    ).rejects.toThrow('boom')
    expect(globalThis.fetch).toBe(original)
  })

  it('refuses a second stub rather than letting one orphan the other', async () => {
    const original = globalThis.fetch
    const settled = await Promise.allSettled([
      withMockHttp([{ url: '/api', body: { n: 1 } }], async () => {
        await fetch('https://acme.test/api')
        // Long enough that the second install would overlap this one.
        await new Promise((resolve) => setTimeout(resolve, 10))
        return 'first'
      }),
      withMockHttp([{ url: '/api', body: { n: 2 } }], () => 'second')
    ])

    expect(settled.map((entry) => entry.status)).toEqual(['fulfilled', 'rejected'])
    const refused = settled[1] as PromiseRejectedResult
    expect(String(refused.reason)).toContain('already serving')
    // The real fetch is back: a dead stub would have been left installed.
    expect(globalThis.fetch).toBe(original)
  })

  it('serves again once the first call is done', async () => {
    await withMockHttp([{ url: '/api' }], () => fetch('https://acme.test/api'))
    const { calls } = await withMockHttp([{ url: '/api' }], () => fetch('https://acme.test/api'))
    expect(calls).toHaveLength(1)
  })

  it('matches a path exactly, so a query string cannot claim a route', async () => {
    const exact = await withMockHttp([{ url: '/api/messages' }], () =>
      fetch('https://acme.test/api/messages?limit=1')
    )
    expect(exact.calls).toHaveLength(1)

    await expect(
      withMockHttp([{ url: '/admin' }], () => fetch('https://acme.test/api?next=/admin'))
    ).rejects.toThrow(/No mock route/)
  })

  it('takes a trailing slash as everything underneath', async () => {
    const { calls } = await withMockHttp([{ url: '/api/' }], () =>
      fetch('https://acme.test/api/messages/42')
    )
    expect(calls).toHaveLength(1)

    // The prefix is a path prefix, not a string one: /apiary is elsewhere.
    await expect(
      withMockHttp([{ url: '/api/' }], () => fetch('https://acme.test/apiary'))
    ).rejects.toThrow(/No mock route/)
  })

  it('lets a pattern see the whole URL, for when the host is the difference', async () => {
    const { calls } = await withMockHttp([{ url: /^https:\/\/auth\.acme\.test\// }], () =>
      fetch('https://auth.acme.test/token')
    )
    expect(calls[0].method).toBe('GET')
  })
})

describe('a check that runs every action against served HTTP', () => {
  const codes = (findings: Awaited<ReturnType<typeof checkConnector>>) =>
    findings.map((item) => item.code)

  it('says nothing about actions until it is asked to run them', async () => {
    const findings = await checkConnector(connector())
    expect(codes(findings)).not.toContain('mock-network-escape')
  })

  it('runs an action on its own declared arguments', async () => {
    const findings = await checkConnector(connector(), {
      mock: true,
      mockRoutes: [{ url: '/api/messages', body: { id: 'm-1' } }]
    })
    expect(codes(findings)).not.toContain('mock-action-failed')
    expect(codes(findings)).not.toContain('mock-network-escape')
  })

  it('refuses an action that reaches past the routes it was given', async () => {
    const findings = await checkConnector(connector(), {
      mock: true,
      mockRoutes: [{ url: '/somewhere-else' }]
    })
    const escape = findings.find((item) => item.code === 'mock-network-escape')
    expect(escape?.level).toBe('error')
  })

  it('fills config from defaults and placeholders, so a templated URL resolves', async () => {
    const declared: ActionDefinition = {
      type: 'create',
      label: 'Create',
      request: {
        method: 'POST',
        url: '{{config.baseUrl}}/v1/items',
        headers: { authorization: 'Bearer {{config.apiToken}}' }
      }
    }
    const sdk = defineConnector({
      id: 'acme',
      name: 'Acme',
      config: [
        { key: 'apiToken', label: 'Token', required: true, secret: true },
        { key: 'baseUrl', label: 'Base URL', default: 'https://api.example.com' }
      ],
      actions: [declared]
    })
    expect(mockConfig(sdk)).toEqual({
      apiToken: 'mock-apiToken',
      baseUrl: 'https://api.example.com'
    })
    const findings = await checkConnector(sdk, {
      mock: true,
      mockRoutes: [{ url: '/v1/items', body: { id: 'i-1' } }]
    })
    expect(codes(findings)).not.toContain('mock-action-failed')
  })

  it('answers everything when no routes were named, so a bare run still proves it runs', async () => {
    const local: ActionDefinition = {
      ...post,
      run: () => ({ id: 'local' })
    }
    const findings = await checkConnector(connector([local]), { mock: true })
    expect(codes(findings)).not.toContain('mock-action-failed')
  })

  it('says so when the stub heard nothing, because it only replaces fetch', async () => {
    const elsewhere: ActionDefinition = {
      ...post,
      // Stands in for a connector that shells out or opens its own socket:
      // whatever it did, the stub did not see it.
      run: () => ({ id: 'from-a-subprocess' })
    }
    const findings = await checkConnector(connector([elsewhere]), { mock: true })
    const unobserved = findings.find((item) => item.code === 'mock-not-observed')
    expect(unobserved?.level).toBe('warn')
    expect(unobserved?.target).toBe('action post')
  })

  it('says nothing of the sort when a call was intercepted', async () => {
    const findings = await checkConnector(connector(), {
      mock: true,
      mockRoutes: [{ url: '/api/messages', body: { id: 'm-1' } }]
    })
    expect(codes(findings)).not.toContain('mock-not-observed')
  })

  it('still calls it an escape when the action rethrew with its own name in front', async () => {
    // What a declarative action does: catch, prefix, rethrow with a cause.
    const wrapping: ActionDefinition = {
      ...post,
      async run() {
        try {
          await fetch('https://acme.test/elsewhere')
          return {}
        } catch (error) {
          throw new Error(`Action post: ${(error as Error).message}`, { cause: error })
        }
      }
    }
    const findings = await checkConnector(connector([wrapping]), {
      mock: true,
      mockRoutes: [{ url: '/api/messages' }]
    })
    const escape = findings.find((item) => item.code === 'mock-network-escape')
    expect(escape?.level).toBe('error')
  })

  it('reports the escape, not the silence, when an action reached past its routes', async () => {
    const findings = await checkConnector(connector(), {
      mock: true,
      mockRoutes: [{ url: '/somewhere-else' }]
    })
    expect(codes(findings)).toContain('mock-network-escape')
    expect(codes(findings)).not.toContain('mock-not-observed')
  })

  it('only warns when an empty reply was all the action had to go on', async () => {
    const strict: ActionDefinition = {
      ...post,
      async run() {
        const response = await fetch('https://acme.test/api/messages')
        const body = (await response.json()) as { id?: string }
        if (!body.id) throw new Error('Acme returned no id')
        return body
      }
    }
    // The default route answers `{}`, which is not a real reply — the
    // connector is not wrong for refusing it.
    const bare = await checkConnector(connector([strict]), { mock: true })
    expect(bare.find((item) => item.code === 'mock-action-failed')?.level).toBe('warn')

    // With routes the caller said what the service returns, so a throw is the
    // connector's to answer for.
    const routed = await checkConnector(connector([strict]), {
      mock: true,
      mockRoutes: [{ url: '/api/', body: {} }]
    })
    expect(routed.find((item) => item.code === 'mock-action-failed')?.level).toBe('error')
  })
})
