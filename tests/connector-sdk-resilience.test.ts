import { describe, expect, it, vi } from 'vitest'
import {
  backoffMs,
  defineConnector,
  nextLink,
  resilientFetch,
  retryAfterMs,
  runAction
} from '../packages/connector-sdk/src/index'
import type { ActionDefinition, PaginationStrategy } from '../packages/connector-sdk/src/types'

/** Answers each call with the next scripted response, and counts the calls. */
function scripted(
  pages: Array<{ body?: unknown; status?: number; headers?: Record<string, string> }>
) {
  let index = 0
  const urls: string[] = []
  const impl = vi.fn(async (url: string | URL | Request) => {
    urls.push(String(url))
    const page = pages[Math.min(index, pages.length - 1)]
    index += 1
    return new Response(page.body === undefined ? '' : JSON.stringify(page.body), {
      status: page.status ?? 200,
      headers: { 'content-type': 'application/json', ...page.headers }
    })
  })
  return { urls, impl: impl as unknown as typeof fetch, calls: () => impl.mock.calls.length }
}

/** A fake clock: records what it was asked to wait rather than waiting. */
function fakeSleep() {
  const waits: number[] = []
  return {
    waits,
    sleep: async (ms: number) => {
      waits.push(ms)
    }
  }
}

const withRequest = (action: Partial<ActionDefinition>) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    actions: [
      {
        type: 'act',
        label: 'Act',
        request: { url: 'https://api.test/t' },
        ...action
      } as ActionDefinition
    ]
  })

describe('what the SDK waits before trying again', () => {
  it('doubles each time, up to a ceiling', () => {
    expect(backoffMs(0, { baseDelayMs: 100 })).toBe(100)
    expect(backoffMs(1, { baseDelayMs: 100 })).toBe(200)
    expect(backoffMs(2, { baseDelayMs: 100 })).toBe(400)
    expect(backoffMs(10, { baseDelayMs: 100, maxDelayMs: 1000 })).toBe(1000)
  })

  it('reads a Retry-After given in seconds or as a date', () => {
    const now = Date.parse('2026-09-02T00:00:00.000Z')
    expect(retryAfterMs('2', now)).toBe(2000)
    expect(retryAfterMs('Wed, 02 Sep 2026 00:00:05 GMT', now)).toBe(5000)
    expect(retryAfterMs(null, now)).toBeUndefined()
    expect(retryAfterMs('soon', now)).toBeUndefined()
  })
})

describe('a call the SDK is allowed to repeat', () => {
  it('tries a rate limit again, waiting exactly as long as it was asked to', async () => {
    const { impl } = scripted([
      { status: 429, headers: { 'retry-after': '3' } },
      { body: { ok: true } }
    ])
    const clock = fakeSleep()
    const fetchImpl = resilientFetch({ fetchImpl: impl, retryable: true, sleep: clock.sleep })

    const response = await fetchImpl('https://api.test/t')

    expect(response.status).toBe(200)
    expect(clock.waits).toEqual([3000])
  })

  it('backs off when the server names no wait of its own', async () => {
    const { impl } = scripted([{ status: 503 }, { status: 503 }, { body: { ok: true } }])
    const clock = fakeSleep()
    const fetchImpl = resilientFetch({
      fetchImpl: impl,
      retryable: true,
      retry: { attempts: 3, baseDelayMs: 100 },
      sleep: clock.sleep
    })

    expect((await fetchImpl('https://api.test/t')).status).toBe(200)
    expect(clock.waits).toEqual([100, 200])
  })

  it('tries again when the network throws, and gives up with that error', async () => {
    const impl = vi.fn(async () => {
      throw new Error('socket hang up')
    }) as unknown as typeof fetch
    const clock = fakeSleep()
    const fetchImpl = resilientFetch({
      fetchImpl: impl,
      retryable: true,
      retry: { attempts: 2, baseDelayMs: 1 },
      sleep: clock.sleep
    })

    await expect(fetchImpl('https://api.test/t')).rejects.toThrow('socket hang up')
    expect(clock.waits).toHaveLength(1)
  })

  it('hands back the failing status once the tries run out', async () => {
    const { impl, calls } = scripted([{ status: 500 }])
    const fetchImpl = resilientFetch({
      fetchImpl: impl,
      retryable: true,
      retry: { attempts: 2, baseDelayMs: 1 },
      sleep: fakeSleep().sleep
    })

    expect((await fetchImpl('https://api.test/t')).status).toBe(500)
    expect(calls()).toBe(2)
  })

  it('leaves an answer that is not a hiccup alone', async () => {
    const { impl, calls } = scripted([{ status: 404, body: { error: 'gone' } }])
    const fetchImpl = resilientFetch({ fetchImpl: impl, retryable: true })

    expect((await fetchImpl('https://api.test/t')).status).toBe(404)
    expect(calls()).toBe(1)
  })
})

describe('a call the SDK must not repeat', () => {
  it('leaves a write alone unless the action says repeating it is safe', async () => {
    const { impl, calls } = scripted([{ status: 500 }])
    await expect(
      runAction(
        withRequest({ request: { method: 'POST', url: 'https://api.test/t' } }),
        'act',
        {},
        { fetchImpl: impl, sleep: fakeSleep().sleep }
      )
    ).rejects.toThrow(/500/)
    expect(calls()).toBe(1)
  })

  it('repeats a write the action marked idempotent', async () => {
    const { impl, calls } = scripted([{ status: 500 }, { body: { ok: true } }])
    const output = await runAction(
      withRequest({ idempotent: true, request: { method: 'PUT', url: 'https://api.test/t' } }),
      'act',
      {},
      { fetchImpl: impl, retry: { attempts: 2, baseDelayMs: 1 }, sleep: fakeSleep().sleep }
    )
    expect(output).toEqual({ ok: true })
    expect(calls()).toBe(2)
  })

  it('repeats a declared read without being asked, because a GET changes nothing', async () => {
    const { impl, calls } = scripted([{ status: 503 }, { body: { ok: true } }])
    await runAction(
      withRequest({}),
      'act',
      {},
      {
        fetchImpl: impl,
        retry: { attempts: 2, baseDelayMs: 1 },
        sleep: fakeSleep().sleep
      }
    )
    expect(calls()).toBe(2)
  })
})

describe('following a source to the end of its pages', () => {
  const paginated = (paginate: PaginationStrategy) =>
    withRequest({ request: { url: 'https://api.test/t', paginate } })

  it('follows a cursor until the source stops handing one back', async () => {
    const { impl, urls } = scripted([
      { body: { data: [1, 2], next: 'c2' } },
      { body: { data: [3], next: null } }
    ])
    const output = await runAction(
      paginated({ kind: 'cursor', cursorPath: 'next', param: 'cursor', itemsPath: 'data' }),
      'act',
      {},
      { fetchImpl: impl }
    )

    expect(output).toEqual({ items: [1, 2, 3] })
    expect(urls[1]).toContain('cursor=c2')
  })

  it('counts pages until one comes back empty', async () => {
    const { impl, urls } = scripted([{ body: [1] }, { body: [2] }, { body: [] }])
    const output = await runAction(
      paginated({ kind: 'page', param: 'page' }),
      'act',
      {},
      { fetchImpl: impl }
    )

    expect(output).toEqual({ items: [1, 2] })
    expect(urls.map((url) => new URL(url).searchParams.get('page'))).toEqual(['1', '2', '3'])
  })

  it('follows the Link header a paged API sends', async () => {
    const { impl, urls } = scripted([
      { body: [1], headers: { link: '<https://api.test/t?page=2>; rel="next"' } },
      { body: [2] }
    ])
    const output = await runAction(paginated({ kind: 'link' }), 'act', {}, { fetchImpl: impl })

    expect(output).toEqual({ items: [1, 2] })
    expect(urls[1]).toBe('https://api.test/t?page=2')
    expect(nextLink('<https://api.test/t?page=2>; rel="next"')).toBe('https://api.test/t?page=2')
    expect(nextLink('<https://api.test/t?page=1>; rel="prev"')).toBeUndefined()
  })

  it('refuses a cursor that does not move rather than looping forever', async () => {
    const { impl } = scripted([{ body: { data: [1], next: 'same' } }])
    await expect(
      runAction(
        paginated({ kind: 'cursor', cursorPath: 'next', param: 'cursor', itemsPath: 'data' }),
        'act',
        {},
        { fetchImpl: impl }
      )
    ).rejects.toThrow(/same page twice/)
  })
})
