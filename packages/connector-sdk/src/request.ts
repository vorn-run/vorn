import { applyPostReceive, valueAt } from './post-receive'
import type { ActionRequest, ConnectorConfig, PaginationStrategy, PostReceiveOp } from './types'

/**
 * Turning a declared request into a real one.
 *
 * A connector's actions are mostly the same shape — put these arguments into a
 * URL, send them, keep part of the answer. Declaring that instead of writing it
 * means the SDK owns the parts every author would otherwise re-implement: URL
 * encoding, dropping arguments nobody supplied, reading the error body when the
 * call fails.
 */

/** How much of a failed response to quote back. Enough to name the cause. */
const MAX_ERROR_BODY = 500

const PLACEHOLDER = /\{\{\s*(args|config)\.([A-Za-z0-9_.-]+)\s*\}\}/g
/** A string that is nothing but one placeholder keeps the value's own type. */
const WHOLE_PLACEHOLDER = /^\{\{\s*(args|config)\.([A-Za-z0-9_.-]+)\s*\}\}$/

export interface RequestScope {
  args: Record<string, unknown>
  config: ConnectorConfig
}

function lookup(source: string, path: string, scope: RequestScope): unknown {
  return valueAt(source === 'args' ? scope.args : scope.config, path)
}

/**
 * How a substituted value is written into its surroundings.
 *
 * A value's meaning depends on where it lands: a path segment has to be
 * escaped, a header has characters it may not contain at all. Passing that
 * decision in means the substitution is made safe once, here, instead of by
 * every author who interpolates an argument.
 */
export type Substitution = (value: string, source: 'args' | 'config') => string

/**
 * Escape a substitution for a URL, but leave one that came from config alone.
 *
 * An argument is somebody else's input: `42?role=admin` or `../../admin` in one
 * would otherwise rewrite the request the connector's own credentials are about
 * to authenticate. Config is the connector's own setting — a base URL is a URL,
 * and escaping it would break the request instead of protecting it.
 */
const intoUrl: Substitution = (value, source) =>
  source === 'config' ? value : encodeURIComponent(value)

/**
 * A header value may not carry a line ending: one would end the header and
 * start whatever the injected text says next.
 */
function intoHeader(name: string): Substitution {
  return (value) => {
    if (/[\r\n]/.test(value)) {
      throw new Error(`Header "${name}" would carry a line ending, which is not allowed`)
    }
    return value
  }
}

/**
 * Resolve `{{args.x}}` and `{{config.y}}` inside a value.
 *
 * A whole-string placeholder keeps the referenced value's own type, so a body
 * can carry a number or an object; a placeholder among other text is rendered
 * into the string. An unset reference resolves to `undefined` on its own and to
 * the empty string when it is part of a larger one, which is what lets an
 * optional argument simply not appear.
 *
 * With a `substitute`, every resolved value passes through it — including a
 * whole-string one, which then arrives as text rather than keeping its type,
 * because a place that needs escaping is a place that holds a string.
 */
export function resolveTemplates(
  value: unknown,
  scope: RequestScope,
  substitute?: Substitution
): unknown {
  if (typeof value === 'string') {
    const whole = WHOLE_PLACEHOLDER.exec(value)
    if (whole) {
      const resolved = lookup(whole[1], whole[2], scope)
      if (substitute === undefined || resolved === undefined || resolved === null) return resolved
      return substitute(String(resolved), whole[1] as 'args' | 'config')
    }
    return value.replace(PLACEHOLDER, (_match, source: string, path: string) => {
      const resolved = lookup(source, path, scope)
      if (resolved === undefined || resolved === null) return ''
      const text = String(resolved)
      return substitute === undefined ? text : substitute(text, source as 'args' | 'config')
    })
  }
  if (Array.isArray(value)) return value.map((entry) => resolveTemplates(entry, scope, substitute))
  if (typeof value === 'object' && value !== null) {
    const out: Record<string, unknown> = {}
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      out[key] = resolveTemplates(entry, scope, substitute)
    }
    return out
  }
  return value
}

/** Resolve each header on its own, so a refusal can name the one at fault. */
function resolveHeaders(
  raw: Record<string, string> | undefined,
  scope: RequestScope
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [name, value] of Object.entries(raw ?? {})) {
    const resolved = resolveTemplates(value, scope, intoHeader(name))
    if (resolved === undefined || resolved === null || resolved === '') continue
    out[name] = String(resolved)
  }
  return out
}

/** Drop entries whose value never resolved, and stringify the rest. */
function stringMap(raw: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  if (typeof raw !== 'object' || raw === null) return out
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (value === undefined || value === null || value === '') continue
    out[key] = String(value)
  }
  return out
}

export interface ResolvedRequest {
  url: string
  method: string
  headers: Record<string, string>
  body?: string
}

/** Build the exact call a declared request makes, with its templates resolved. */
export function resolveRequest(request: ActionRequest, scope: RequestScope): ResolvedRequest {
  const method = (request.method ?? 'GET').toUpperCase()
  const rawUrl = resolveTemplates(request.url, scope, intoUrl)
  if (typeof rawUrl !== 'string' || rawUrl.trim() === '') {
    throw new Error('Request has no URL once its templates are resolved')
  }

  let url: URL
  try {
    url = new URL(rawUrl)
  } catch {
    // Says what it actually tried to call: the declared template rarely names
    // the problem, and the resolved string always does.
    throw new Error(`Request URL is not a URL once its templates are resolved: "${rawUrl}"`)
  }
  for (const [key, value] of Object.entries(stringMap(resolveTemplates(request.query, scope)))) {
    url.searchParams.set(key, value)
  }

  const headers = resolveHeaders(request.headers, scope)
  const resolved: ResolvedRequest = { url: url.toString(), method, headers }

  if (request.body !== undefined && method !== 'GET' && method !== 'HEAD') {
    const body = resolveTemplates(request.body, scope)
    if (body !== undefined) {
      resolved.body = typeof body === 'string' ? body : JSON.stringify(body)
      // Only defaulted: an author sending form-encoded or XML said so already.
      if (!Object.keys(headers).some((key) => key.toLowerCase() === 'content-type')) {
        resolved.headers['content-type'] =
          typeof body === 'string' ? 'text/plain' : 'application/json'
      }
    }
  }

  return resolved
}

/** Read a response as JSON when it says it is, and as text otherwise. */
async function readBody(response: Response): Promise<unknown> {
  const text = await response.text()
  if (text === '') return undefined
  const type = response.headers.get('content-type') ?? ''
  if (!type.includes('json')) return text
  try {
    return JSON.parse(text)
  } catch {
    // A body that claims to be JSON and is not is the upstream's mistake, and
    // the raw text says more about it than a parse error would.
    return text
  }
}

function describeFailure(response: Response, body: unknown): string {
  const detail = typeof body === 'string' ? body : body === undefined ? '' : JSON.stringify(body)
  const quoted = detail.length > MAX_ERROR_BODY ? `${detail.slice(0, MAX_ERROR_BODY)}…` : detail
  return `Request failed with ${response.status} ${response.statusText}${quoted ? `: ${quoted}` : ''}`
}

export interface SendOptions {
  fetchImpl: typeof fetch
}

/** Send one resolved request and read its body, throwing on a failed status. */
export async function sendRequest(
  resolved: ResolvedRequest,
  options: SendOptions
): Promise<{ response: Response; body: unknown }> {
  const response = await options.fetchImpl(resolved.url, {
    method: resolved.method,
    headers: resolved.headers,
    ...(resolved.body !== undefined && { body: resolved.body })
  })
  const body = await readBody(response)
  if (!response.ok) throw new Error(describeFailure(response, body))
  return { response, body }
}

/**
 * The action's result, as Vorn stores it.
 *
 * A step's output is a record, so a response that is a list becomes `items` —
 * the name the rest of this SDK already uses for one — and any other bare
 * value becomes `result`.
 */
export function asOutput(value: unknown): Record<string, unknown> {
  if (Array.isArray(value)) return { items: value }
  if (typeof value === 'object' && value !== null) return value as Record<string, unknown>
  return value === undefined ? {} : { result: value }
}

/** Longest chain of pages a declared request will follow before calling it a bug. */
export const MAX_REQUEST_PAGES = 100

const LINK_NEXT = /<([^>]+)>\s*;[^,]*\brel\s*=\s*"?next"?/i

/** The URL of the next page, as a paged HTTP API states it in its `Link` header. */
export function nextLink(header: string | null): string | undefined {
  const match = header === null ? null : LINK_NEXT.exec(header)
  return match ? match[1] : undefined
}

/** The list a page carries, at `itemsPath` or as the whole body. */
function pageItems(body: unknown, itemsPath: string | undefined): unknown[] | undefined {
  const value = itemsPath === undefined ? body : valueAt(body, itemsPath)
  return Array.isArray(value) ? value : undefined
}

/**
 * Follow a declared request to the end of its pages.
 *
 * Stops when the source runs out, when it stops moving — a cursor that repeats
 * would otherwise loop forever — or at a bound, so a paging bug shows up as an
 * error rather than as a step that never finishes.
 */
async function collectPages(
  request: ActionRequest,
  strategy: PaginationStrategy,
  scope: RequestScope,
  options: SendOptions
): Promise<unknown[]> {
  const collected: unknown[] = []
  const seen = new Set<string>()
  let page = strategy.kind === 'page' ? (strategy.startPage ?? 1) : 0
  let cursor: string | undefined
  let nextUrl: string | undefined

  for (let index = 0; index < MAX_REQUEST_PAGES; index++) {
    const resolved = resolveRequest(request, scope)
    if (nextUrl !== undefined) resolved.url = nextUrl
    if (strategy.kind === 'cursor' && cursor !== undefined) {
      const url = new URL(resolved.url)
      url.searchParams.set(strategy.param, cursor)
      resolved.url = url.toString()
    }
    if (strategy.kind === 'page') {
      const url = new URL(resolved.url)
      url.searchParams.set(strategy.param, String(page))
      resolved.url = url.toString()
    }

    if (seen.has(resolved.url)) {
      throw new Error(`Request for ${request.url} asked for the same page twice`)
    }
    seen.add(resolved.url)

    const { response, body } = await sendRequest(resolved, options)
    const items = pageItems(body, strategy.itemsPath)
    // A page that is not a list ends the walk: a source that answered with an
    // object has nothing left to concatenate.
    if (items === undefined) return collected
    collected.push(...items)
    if (items.length === 0) return collected

    if (strategy.kind === 'cursor') {
      const next = valueAt(body, strategy.cursorPath)
      if (next === undefined || next === null || next === '') return collected
      cursor = String(next)
      continue
    }
    if (strategy.kind === 'link') {
      nextUrl = nextLink(response.headers.get('link'))
      if (nextUrl === undefined) return collected
      continue
    }
    page += 1
  }

  throw new Error(`Request for ${request.url} exceeded ${MAX_REQUEST_PAGES} pages`)
}

/** Run a declared request end to end: resolve, send, follow its pages, reshape. */
export async function executeRequest(
  request: ActionRequest,
  postReceive: PostReceiveOp[] | undefined,
  scope: RequestScope,
  options: SendOptions
): Promise<Record<string, unknown>> {
  if (request.paginate) {
    const items = await collectPages(request, request.paginate, scope, options)
    return asOutput(applyPostReceive(items, postReceive))
  }
  const { body } = await sendRequest(resolveRequest(request, scope), options)
  return asOutput(applyPostReceive(body, postReceive))
}
