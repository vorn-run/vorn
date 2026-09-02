import { applyPostReceive, valueAt } from './post-receive'
import type { ActionRequest, ConnectorConfig, PostReceiveOp } from './types'

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
 * Resolve `{{args.x}}` and `{{config.y}}` inside a value.
 *
 * A whole-string placeholder keeps the referenced value's own type, so a body
 * can carry a number or an object; a placeholder among other text is rendered
 * into the string. An unset reference resolves to `undefined` on its own and to
 * the empty string when it is part of a larger one, which is what lets an
 * optional argument simply not appear.
 */
export function resolveTemplates(value: unknown, scope: RequestScope): unknown {
  if (typeof value === 'string') {
    const whole = WHOLE_PLACEHOLDER.exec(value)
    if (whole) return lookup(whole[1], whole[2], scope)
    return value.replace(PLACEHOLDER, (_match, source: string, path: string) => {
      const resolved = lookup(source, path, scope)
      return resolved === undefined || resolved === null ? '' : String(resolved)
    })
  }
  if (Array.isArray(value)) return value.map((entry) => resolveTemplates(entry, scope))
  if (typeof value === 'object' && value !== null) {
    const out: Record<string, unknown> = {}
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      out[key] = resolveTemplates(entry, scope)
    }
    return out
  }
  return value
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
  const rawUrl = resolveTemplates(request.url, scope)
  if (typeof rawUrl !== 'string' || rawUrl.trim() === '') {
    throw new Error('Request has no URL once its templates are resolved')
  }

  const url = new URL(rawUrl)
  for (const [key, value] of Object.entries(stringMap(resolveTemplates(request.query, scope)))) {
    url.searchParams.set(key, value)
  }

  const headers = stringMap(resolveTemplates(request.headers, scope))
  const resolved: ResolvedRequest = { url: url.toString(), method, headers }

  if (request.body !== undefined && method !== 'GET' && method !== 'HEAD') {
    const body = resolveTemplates(request.body, scope)
    if (body !== undefined) {
      resolved.body = typeof body === 'string' ? body : JSON.stringify(body)
      // Only defaulted: an author sending form-encoded or XML said so already.
      if (!Object.keys(headers).some((key) => key.toLowerCase() === 'content-type')) {
        headers['content-type'] = typeof body === 'string' ? 'text/plain' : 'application/json'
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

/** Run a declared request end to end: resolve, send, reshape. */
export async function executeRequest(
  request: ActionRequest,
  postReceive: PostReceiveOp[] | undefined,
  scope: RequestScope,
  options: SendOptions
): Promise<Record<string, unknown>> {
  const { body } = await sendRequest(resolveRequest(request, scope), options)
  return asOutput(applyPostReceive(body, postReceive))
}
