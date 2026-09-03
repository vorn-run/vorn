import { describe, expect, it, vi } from 'vitest'
import {
  applyPostReceive,
  defineConnector,
  resolveRequest,
  runAction
} from '../packages/connector-sdk/src/index'
import type { ActionDefinition, PostReceiveOp } from '../packages/connector-sdk/src/types'

/** A fetch that answers every call with the same JSON, and records the calls. */
function fakeFetch(
  body: unknown,
  init: { status?: number; headers?: Record<string, string> } = {}
) {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const impl = vi.fn(async (url: string | URL | Request, requestInit?: RequestInit) => {
    calls.push({ url: String(url), init: requestInit ?? {} })
    return new Response(typeof body === 'string' ? body : JSON.stringify(body), {
      status: init.status ?? 200,
      headers: { 'content-type': 'application/json', ...init.headers }
    })
  })
  return { calls, impl: impl as unknown as typeof fetch }
}

const declared = (action: Partial<ActionDefinition>) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    config: [{ key: 'token', label: 'Token' }],
    actions: [
      {
        type: 'post',
        label: 'Post',
        request: { url: 'https://api.test/things' },
        ...action
      } as ActionDefinition
    ]
  })

describe('a request an action declares', () => {
  it('fills its URL, query and headers from the arguments and the config', () => {
    const resolved = resolveRequest(
      {
        method: 'POST',
        url: 'https://api.test/rooms/{{args.room}}/messages',
        query: { limit: '{{args.limit}}', cursor: '{{args.cursor}}' },
        headers: { authorization: 'Bearer {{config.token}}' }
      },
      { args: { room: 'general', limit: 10 }, config: { token: 'sk-1' } }
    )

    expect(resolved.url).toBe('https://api.test/rooms/general/messages?limit=10')
    expect(resolved.headers.authorization).toBe('Bearer sk-1')
    expect(resolved.method).toBe('POST')
  })

  it('keeps a whole-placeholder value as itself, so a body can carry a number', () => {
    const resolved = resolveRequest(
      { method: 'POST', url: 'https://api.test/t', body: { count: '{{args.count}}', to: 'x' } },
      { args: { count: 7 }, config: {} }
    )
    expect(JSON.parse(resolved.body as string)).toEqual({ count: 7, to: 'x' })
    expect(resolved.headers['content-type']).toBe('application/json')
  })

  it('leaves out an argument nobody supplied rather than sending an empty one', () => {
    const resolved = resolveRequest(
      { url: 'https://api.test/t', query: { after: '{{args.after}}', q: 'ok' } },
      { args: {}, config: {} }
    )
    expect(resolved.url).toBe('https://api.test/t?q=ok')
  })

  it('sends the call and returns what came back', async () => {
    const { calls, impl } = fakeFetch({ id: '1', ok: true })
    const output = await runAction(
      declared({ request: { method: 'POST', url: 'https://api.test/things' } }),
      'post',
      {},
      { fetchImpl: impl }
    )

    expect(output).toEqual({ id: '1', ok: true })
    expect(calls[0].init.method).toBe('POST')
  })

  it('reports a failed status with what the body said about it', async () => {
    const { impl } = fakeFetch({ error: 'no such room' }, { status: 404 })
    await expect(runAction(declared({}), 'post', {}, { fetchImpl: impl })).rejects.toThrow(
      /404.*no such room/
    )
  })

  it('names a list "items" and a bare value "result"', async () => {
    const list = await runAction(declared({}), 'post', {}, { fetchImpl: fakeFetch([1, 2]).impl })
    expect(list).toEqual({ items: [1, 2] })

    const scalar = await runAction(declared({}), 'post', {}, { fetchImpl: fakeFetch('"hi"').impl })
    expect(scalar).toEqual({ result: 'hi' })
  })
})

describe('an argument that tries to rewrite the request', () => {
  const resolve = (url: string, args: Record<string, unknown>) =>
    resolveRequest({ url }, { args, config: { baseUrl: 'https://api.test' } }).url

  it('escapes a query string smuggled into a path segment', () => {
    expect(resolve('https://api.test/things/{{args.id}}', { id: '42?role=admin' })).toBe(
      'https://api.test/things/42%3Frole%3Dadmin'
    )
  })

  it('escapes a fragment, so nothing after it is dropped', () => {
    expect(resolve('https://api.test/things/{{args.id}}', { id: 'a#b' })).toBe(
      'https://api.test/things/a%23b'
    )
  })

  it('escapes traversal, so the path cannot climb out of where it was put', () => {
    expect(resolve('https://api.test/things/{{args.id}}', { id: '../../admin' })).toBe(
      'https://api.test/things/..%2F..%2Fadmin'
    )
  })

  it('leaves the connector own settings alone, so a base URL stays a URL', () => {
    expect(resolve('{{config.baseUrl}}/v1/items', {})).toBe('https://api.test/v1/items')
    expect(resolve('{{config.baseUrl}}', {})).toBe('https://api.test/')
  })

  it('escapes a whole-placeholder argument too, so it cannot become the host', () => {
    // Escaped, the borrowed URL is no longer a URL — which is the point.
    expect(() => resolve('{{args.target}}', { target: 'https://elsewhere.test/x' })).toThrow(
      /not a URL once its templates are resolved/
    )
  })

  it('refuses a header value carrying a line ending, and names the header', () => {
    expect(() =>
      resolveRequest(
        { url: 'https://api.test/t', headers: { 'x-note': 'note: {{args.note}}' } },
        { args: { note: 'ok\r\nAuthorization: Bearer stolen' }, config: {} }
      )
    ).toThrow(/Header "x-note" would carry a line ending/)
  })

  it('still sends an ordinary argument unmangled', () => {
    expect(resolve('https://api.test/rooms/{{args.room}}', { room: 'general' })).toBe(
      'https://api.test/rooms/general'
    )
  })
})

describe('reshaping what came back', () => {
  const envelope = {
    data: [
      { id: 1, name: 'One', kind: 'issue', noise: true },
      { id: 2, name: 'Two', kind: 'note', noise: true }
    ],
    meta: { total: 2 }
  }

  const run = (ops: PostReceiveOp[]) => applyPostReceive(envelope, ops)

  it('unwraps an envelope', () => {
    expect(run([{ op: 'flatten', path: 'data' }])).toEqual(envelope.data)
  })

  it('keeps only the keys worth keeping, through a list', () => {
    expect(run([{ op: 'pick', path: 'data', keys: ['id', 'name'] }])).toEqual({
      ...envelope,
      data: [
        { id: 1, name: 'One' },
        { id: 2, name: 'Two' }
      ]
    })
  })

  it('renames a key in place rather than moving it to the end', () => {
    const renamed = run([{ op: 'rename', path: 'data', from: 'name', to: 'title' }]) as {
      data: Array<Record<string, unknown>>
    }
    expect(Object.keys(renamed.data[0])).toEqual(['id', 'title', 'kind', 'noise'])
  })

  it('filters a list by a field', () => {
    const filtered = run([{ op: 'filter', path: 'data', key: 'kind', equals: 'issue' }]) as {
      data: unknown[]
    }
    expect(filtered.data).toHaveLength(1)
  })

  it('maps a sub-pipeline over every entry', () => {
    const mapped = run([{ op: 'map', path: 'data', ops: [{ op: 'pick', keys: ['id'] }] }]) as {
      data: unknown[]
    }
    expect(mapped.data).toEqual([{ id: 1 }, { id: 2 }])
  })

  it('composes left to right, which is how an envelope becomes a list of items', () => {
    expect(
      run([
        { op: 'flatten', path: 'data' },
        { op: 'filter', key: 'kind', equals: 'issue' },
        { op: 'pick', keys: ['id', 'name'] },
        { op: 'rename', from: 'name', to: 'title' }
      ])
    ).toEqual([{ id: 1, title: 'One' }])
  })

  it('leaves a value alone when the path names nothing, and refuses prototype keys', () => {
    expect(applyPostReceive({ a: 1 }, [{ op: 'pick', path: 'nope', keys: ['x'] }])).toEqual({
      a: 1
    })
    expect(applyPostReceive({ a: 1 }, [{ op: 'flatten', path: '__proto__' }])).toBeUndefined()
  })

  it('runs the ops a declared action carries', async () => {
    const { impl } = fakeFetch(envelope)
    const output = await runAction(
      declared({
        request: { url: 'https://api.test/things' },
        postReceive: [
          { op: 'flatten', path: 'data' },
          { op: 'pick', keys: ['id'] }
        ]
      }),
      'post',
      {},
      { fetchImpl: impl }
    )
    expect(output).toEqual({ items: [{ id: 1 }, { id: 2 }] })
  })
})

describe('what an action is allowed to be', () => {
  const build = (action: Record<string, unknown>) =>
    defineConnector({
      id: 'acme',
      name: 'Acme',
      actions: [{ type: 'post', label: 'Post', ...action } as unknown as ActionDefinition]
    })

  it('refuses one that is both written and declared', () => {
    expect(() => build({ run: () => ({}), request: { url: 'https://api.test' } })).toThrow(
      /pick one/
    )
  })

  it('refuses one that is neither', () => {
    expect(() => build({})).toThrow(/missing a run\(\) implementation or a request/)
  })

  it('refuses a request with no URL, and postReceive with nothing to reshape', () => {
    expect(() => build({ request: { url: '  ' } })).toThrow(/no URL/)
    expect(() => build({ run: () => ({}), postReceive: [] })).toThrow(/no request/)
  })

  it('says so when one built in plain JS reaches the runtime as neither', async () => {
    // `defineConnector` refuses this; a hand-built object passed straight to
    // `runAction` has never been through it.
    const handBuilt = {
      id: 'acme',
      name: 'Acme',
      version: '0.0.0',
      config: [],
      triggers: [],
      actions: [{ type: 'post', label: 'Post' }]
    } as unknown as Parameters<typeof runAction>[0]

    await expect(runAction(handBuilt, 'post', {})).rejects.toThrow(
      /neither a run\(\) implementation nor a request/
    )
  })
})
