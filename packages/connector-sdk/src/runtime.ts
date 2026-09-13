import { pollWithDedupe } from './dedupe'
import { ActionArgumentError } from './errors'
import { normalizeItems } from './normalize'
import { executeRequest } from './request'
import { resilientFetch, type RetryPolicy } from './resilience'
import { createSessionFetch } from './session'
import type {
  ActionInputOption,
  Connector,
  ConnectorConfig,
  NormalizedItem,
  PollContext,
  SessionContext
} from './types'

export interface PollPage {
  items: NormalizedItem[]
  nextCursor?: string
  hasMore: boolean
}

export interface RunPollOptions {
  config?: ConnectorConfig
  since?: string
  cursor?: string
  limit?: number
  now?: () => string
  /** Replaced by the harness and by tests; defaults to the global fetch. */
  fetchImpl?: typeof fetch
  /** Replaced by the harness and by tests; defaults to the signed-in window Vorn serves. */
  sessionFetchImpl?: typeof fetch
  /** The key Vorn gave this tool call, carried on each request through the window. */
  sessionCall?: string
  retry?: RetryPolicy
  /** Replaced in tests so backoff costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

type SessionOptions = Pick<RunPollOptions, 'sessionFetchImpl' | 'sessionCall' | 'retry' | 'sleep'>

/** Wrap a fetch with the SDK's retries, as far as the caller says a repeat is safe. */
function wrap(
  fetchImpl: typeof fetch,
  options: Pick<RunPollOptions, 'retry' | 'sleep'>,
  retryable: boolean
): typeof fetch {
  return resilientFetch({
    fetchImpl,
    retryable,
    ...(options.retry !== undefined && { retry: options.retry }),
    ...(options.sleep !== undefined && { sleep: options.sleep })
  })
}

/** The signed-in window, handed only to a connector that signs in through one. */
function sessionFor(
  connector: Connector,
  options: SessionOptions,
  retryable: boolean
): SessionContext | undefined {
  if (connector.auth?.rung !== 'browser') return undefined
  const fetchImpl =
    options.sessionFetchImpl ??
    createSessionFetch(options.sessionCall ? { call: options.sessionCall } : {})
  return { fetch: wrap(fetchImpl, options, retryable) }
}

/** Longest chain of pages `drainPoll` will follow before calling it a bug. */
export const MAX_POLL_PAGES = 1_000

/**
 * Run one poll page and normalize it. Shared by the stdio server, the CLI and
 * the test harness so all three observe exactly what Vorn will observe.
 */
export async function runPoll(
  connector: Connector,
  triggerType: string,
  options: RunPollOptions = {}
): Promise<PollPage> {
  const trigger = connector.triggers.find((entry) => entry.type === triggerType)
  if (!trigger) {
    throw new Error(`Connector ${connector.id} has no trigger "${triggerType}"`)
  }

  const now = options.now ?? (() => new Date().toISOString())
  const polledAt = now()
  const session = sessionFor(connector, options, true)
  const context: PollContext = {
    config: options.config ?? {},
    ...(options.since !== undefined && { since: options.since }),
    ...(options.cursor !== undefined && { cursor: options.cursor }),
    ...(options.limit !== undefined && { limit: options.limit }),
    now,
    // A poll only reads, so every failure it meets is worth trying again.
    fetch: wrap(options.fetchImpl ?? globalThis.fetch, options, true),
    ...(session && { session })
  }

  const outcome =
    typeof trigger.poll === 'function'
      ? await trigger.poll(context)
      : await pollWithDedupe(trigger, context)
  if (!outcome || !Array.isArray(outcome.items)) {
    throw new Error(`Trigger ${triggerType} did not return an items array`)
  }
  if (outcome.hasMore && !outcome.nextCursor) {
    throw new Error(`Trigger ${triggerType} reported more pages without a nextCursor`)
  }

  return {
    items: normalizeItems(outcome.items, polledAt),
    ...(outcome.nextCursor !== undefined && { nextCursor: outcome.nextCursor }),
    hasMore: outcome.hasMore === true
  }
}

/**
 * Follow `hasMore` to the end of a trigger's backlog. Mirrors how Vorn drains
 * a connector, including its refusal to follow a cursor that does not move —
 * so an author sees the infinite loop in a unit test instead of in the app.
 */
export async function drainPoll(
  connector: Connector,
  triggerType: string,
  options: RunPollOptions = {}
): Promise<NormalizedItem[]> {
  const collected: NormalizedItem[] = []
  let cursor = options.cursor
  for (let page = 0; page < MAX_POLL_PAGES; page++) {
    const result = await runPoll(connector, triggerType, {
      ...options,
      ...(cursor !== undefined && { cursor })
    })
    collected.push(...result.items)
    if (!result.hasMore) return collected
    if (result.nextCursor === cursor) {
      throw new Error(`Trigger ${triggerType} did not advance its cursor`)
    }
    cursor = result.nextCursor
  }
  throw new Error(`Trigger ${triggerType} exceeded ${MAX_POLL_PAGES} pages`)
}

export interface RunActionOptions {
  config?: ConnectorConfig
  now?: () => string
  /** Replaced by the harness and by tests; defaults to the global fetch. */
  fetchImpl?: typeof fetch
  /** Replaced by the harness and by tests; defaults to the signed-in window Vorn serves. */
  sessionFetchImpl?: typeof fetch
  /** The key Vorn gave this tool call, carried on each request through the window. */
  sessionCall?: string
  retry?: RetryPolicy
  /** Replaced in tests so backoff costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

/** Methods that change nothing, so repeating one cannot do a thing twice. */
const SAFE_METHODS = new Set(['GET', 'HEAD'])

/**
 * Ask a connector what one of its dynamic fields can be.
 *
 * Listing choices only reads, so it retries like a poll does. A bare string is
 * taken as a choice that shows itself, which is the common case.
 */
export async function runOptions(
  connector: Connector,
  name: string,
  options: RunActionOptions = {}
): Promise<ActionInputOption[]> {
  const loader = connector.options?.[name]
  if (!loader) {
    throw new Error(`Connector ${connector.id} serves no options set "${name}"`)
  }

  const session = sessionFor(connector, options, true)
  const loaded = await loader({
    config: options.config ?? {},
    now: options.now ?? (() => new Date().toISOString()),
    fetch: wrap(options.fetchImpl ?? globalThis.fetch, options, true),
    ...(session && { session })
  })

  if (!Array.isArray(loaded)) {
    throw new Error(`Options set "${name}" did not return an array`)
  }
  return loaded.map((entry) =>
    typeof entry === 'string' ? { value: entry } : { ...entry, value: String(entry.value) }
  )
}

/** How much of a bad value to quote back, so an error names it without a wall of text. */
const MAX_QUOTED_VALUE = 80

function quote(value: string): string {
  return value.length > MAX_QUOTED_VALUE ? `${value.slice(0, MAX_QUOTED_VALUE)}…` : value
}

const shown = (value: unknown): string =>
  typeof value === 'string' ? `"${quote(value)}"` : quote(JSON.stringify(value) ?? String(value))

// A typed value is taken as it is; text rendered from a template is read as the declared type.
function coerceArg(value: unknown, type: string | undefined): unknown {
  if (type === 'number') {
    if (typeof value === 'number' && Number.isFinite(value)) return value
    const parsed = typeof value === 'string' ? Number(value) : Number.NaN
    if (Number.isNaN(parsed)) throw new Error(`Expected a number, got ${shown(value)}`)
    return parsed
  }
  if (type === 'boolean') {
    if (typeof value === 'boolean') return value
    if (value === 'true') return true
    if (value === 'false') return false
    throw new Error(`Expected a boolean, got ${shown(value)}`)
  }
  if (type === 'json') {
    if (typeof value !== 'string') return value
    try {
      return JSON.parse(value)
    } catch {
      throw new Error(`Expected JSON, got ${shown(value)}`)
    }
  }
  // Whatever a text field is handed is still text: a number or flag as written, a list or object as JSON.
  if (typeof value === 'string') return value
  return typeof value === 'object' ? JSON.stringify(value) : String(value)
}

/** Run an action with its declared inputs validated and read as their declared types. */
export async function runAction(
  connector: Connector,
  actionType: string,
  args: Record<string, unknown>,
  options: RunActionOptions = {}
): Promise<Record<string, unknown>> {
  const action = connector.actions.find((entry) => entry.type === actionType)
  if (!action) {
    throw new Error(`Connector ${connector.id} has no action "${actionType}"`)
  }

  const coerced: Record<string, unknown> = { ...args }
  for (const input of action.inputs ?? []) {
    const value = coerced[input.key]
    if (value === undefined || value === null || value === '') {
      if (input.required) {
        throw new ActionArgumentError(input.key, `Action ${actionType} requires "${input.key}"`)
      }
      delete coerced[input.key]
      continue
    }
    try {
      coerced[input.key] = coerceArg(value, input.type)
    } catch (error) {
      throw new ActionArgumentError(
        input.key,
        `Action ${actionType} argument "${input.key}": ${
          error instanceof Error ? error.message : String(error)
        }`
      )
    }
  }

  const config = options.config ?? {}
  // Repeating a write invents a second one, so a retry needs the action's word
  // that it is safe — except for a declared read, which says so by its method.
  const method = (action.request?.method ?? 'GET').toUpperCase()
  const retryable =
    action.idempotent === true || (action.request !== undefined && SAFE_METHODS.has(method))
  const fetchImpl = wrap(options.fetchImpl ?? globalThis.fetch, options, retryable)
  const session = sessionFor(connector, options, retryable)

  if (action.request !== undefined) {
    try {
      // A declared call of a browser connector is one to its signed-in service.
      return await executeRequest(
        action.request,
        action.postReceive,
        { args: coerced, config },
        { fetchImpl: session?.fetch ?? fetchImpl }
      )
    } catch (error) {
      // Which action failed is the first thing a reader needs; the message
      // underneath already says what about it went wrong.
      throw new Error(
        `Action ${actionType}: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error }
      )
    }
  }
  // `defineConnector` rules this out; a connector hand-built in plain JS and
  // passed straight here has not been through it.
  if (typeof action.run !== 'function') {
    throw new Error(`Action ${actionType} has neither a run() implementation nor a request`)
  }

  const output = await action.run(coerced, {
    config,
    now: options.now ?? (() => new Date().toISOString()),
    fetch: fetchImpl,
    ...(session && { session })
  })
  return output ?? {}
}
