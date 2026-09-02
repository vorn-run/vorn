import { drainPoll, runAction, runPoll, type PollPage, type RunPollOptions } from './runtime'
import { connectorManifest, type ConnectorManifest } from './setup'
import type { Connector, ConnectorConfig, NormalizedItem } from './types'

export interface HarnessOptions {
  config?: ConnectorConfig
  /** Fixed clock, so `updatedAt` defaults and cursors are deterministic. */
  now?: () => string
  /** Answer the connector's calls from the test rather than the network. */
  fetchImpl?: typeof fetch
  /** Fake clock for backoff, so a retry test costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

/** One reply the stub will serve, matched in the order the routes were given. */
export interface MockRoute {
  /**
   * A string names a path: `/api/messages` matches that path and nothing else,
   * and a trailing slash makes it a prefix — `/api/` matches everything under
   * it. A pattern is tested against the whole URL, for the times host or query
   * is what tells two calls apart.
   */
  url: string | RegExp
  /** Matched case-insensitively; absent matches any method. */
  method?: string
  /** Defaults to 200. */
  status?: number
  /** Serialized as JSON unless it is already a string. */
  body?: unknown
  headers?: Record<string, string>
}

/** What the connector asked for, in the order it asked. */
export interface MockCall {
  method: string
  url: string
  body?: string
}

export interface MockRun<T> {
  result: T
  calls: MockCall[]
}

/**
 * Whether a route answers this call.
 *
 * A string is compared against the path rather than searched for in the whole
 * URL: `/api` should not be answered by a route because the query string
 * happened to mention it, and a connector's own host is not something a stub
 * should match by accident.
 */
function matches(route: MockRoute, method: string, url: string): boolean {
  if (route.method && route.method.toUpperCase() !== method) return false
  if (route.url instanceof RegExp) return route.url.test(url)

  let pathname: string
  try {
    pathname = new URL(url).pathname
  } catch {
    // Not a URL the platform can parse; nothing about it can be trusted.
    return false
  }
  return route.url.endsWith('/') ? pathname.startsWith(route.url) : pathname === route.url
}

function reply(route: MockRoute): Response {
  const body = typeof route.body === 'string' ? route.body : JSON.stringify(route.body ?? {})
  return new Response(body, {
    status: route.status ?? 200,
    headers: { 'content-type': 'application/json', ...route.headers }
  })
}

export interface ConnectorHarness {
  poll(triggerType: string, options?: RunPollOptions): Promise<PollPage>
  drain(triggerType: string, options?: RunPollOptions): Promise<NormalizedItem[]>
  execute(actionType: string, args?: Record<string, unknown>): Promise<Record<string, unknown>>
  manifest(): ConnectorManifest
  /**
   * Poll repeatedly the way Vorn does — carrying the newest `updatedAt`
   * forward as the watermark — and return only items a real installation
   * would treat as new. Catches the classic connector bug where a poll
   * ignores its lower bound and re-delivers the same backlog forever.
   */
  pollTwice(triggerType: string, options?: RunPollOptions): Promise<NormalizedItem[]>
  /**
   * Run something with every HTTP request answered from `routes` instead of
   * the network. A request no route matches is refused rather than served, so
   * a test says which call escaped rather than reaching a real service.
   */
  withMockHttp<T>(routes: MockRoute[], body: () => Promise<T> | T): Promise<MockRun<T>>
}

/** Whether a stub is already installed, so a second one cannot orphan the first. */
let serving = false

/**
 * Serve a connector's HTTP from a list of replies, in-process.
 *
 * Swapping `fetch` rather than opening a socket keeps a conformance run
 * hermetic: no port, no ordering between tests, and the same code path the
 * connector uses against the real service.
 *
 * One at a time, deliberately. Two overlapping installs share one global: the
 * first to finish restores the real `fetch` under the second — whose calls
 * then reach the network — and the second restores the first's dead stub
 * permanently. Refusing is the only outcome that cannot corrupt the process.
 */
export async function withMockHttp<T>(
  routes: MockRoute[],
  body: () => Promise<T> | T
): Promise<MockRun<T>> {
  if (serving) {
    throw new Error(
      'withMockHttp is already serving; give one call every route it needs rather than installing a second stub'
    )
  }
  serving = true
  const calls: MockCall[] = []
  const original = globalThis.fetch
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    calls.push({
      method,
      url,
      ...(typeof init?.body === 'string' && { body: init.body })
    })
    const route = routes.find((candidate) => matches(candidate, method, url))
    if (!route) throw new Error(`No mock route for ${method} ${url}`)
    return reply(route)
  }) as typeof fetch

  try {
    return { result: await body(), calls }
  } finally {
    globalThis.fetch = original
    serving = false
  }
}

/**
 * Run a connector in-process, exactly as the MCP server would, without
 * spawning anything. Authors get real assertions in a plain unit test.
 */
export function createConnectorHarness(
  connector: Connector,
  harnessOptions: HarnessOptions = {}
): ConnectorHarness {
  const defaults = (options: RunPollOptions = {}): RunPollOptions => ({
    ...(harnessOptions.config && { config: harnessOptions.config }),
    ...(harnessOptions.now && { now: harnessOptions.now }),
    ...(harnessOptions.fetchImpl && { fetchImpl: harnessOptions.fetchImpl }),
    ...(harnessOptions.sleep && { sleep: harnessOptions.sleep }),
    ...options
  })

  return {
    poll: (triggerType, options) => runPoll(connector, triggerType, defaults(options)),
    drain: (triggerType, options) => drainPoll(connector, triggerType, defaults(options)),
    execute: (actionType, args = {}) =>
      runAction(connector, actionType, args, {
        ...(harnessOptions.config && { config: harnessOptions.config }),
        ...(harnessOptions.now && { now: harnessOptions.now }),
        ...(harnessOptions.fetchImpl && { fetchImpl: harnessOptions.fetchImpl }),
        ...(harnessOptions.sleep && { sleep: harnessOptions.sleep })
      }),
    manifest: () => connectorManifest(connector),
    async pollTwice(triggerType, options) {
      const first = await drainPoll(connector, triggerType, defaults(options))
      const watermark = first.reduce<string | undefined>(
        (newest, item) =>
          newest === undefined || item.updatedAt > newest ? item.updatedAt : newest,
        options?.since
      )
      const second = await drainPoll(
        connector,
        triggerType,
        defaults({ ...options, ...(watermark !== undefined && { since: watermark }) })
      )
      // Vorn drops anything at or before the watermark, so only strictly
      // newer items count as redelivered work.
      return watermark === undefined ? second : second.filter((item) => item.updatedAt > watermark)
    },
    withMockHttp
  }
}
