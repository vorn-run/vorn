import { pollWithDedupe } from './dedupe'
import { normalizeItems } from './normalize'
import { executeRequest } from './request'
import { resilientFetch, type RetryPolicy } from './resilience'
import type { Connector, ConnectorConfig, NormalizedItem, PollContext } from './types'

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
  retry?: RetryPolicy
  /** Replaced in tests so backoff costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

/** Longest chain of pages `drainPoll` will follow before calling it a bug. */
export const MAX_POLL_PAGES = 1_000

/**
 * Run one poll page and normalize it. Shared by the MCP server, the CLI and
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
  const context: PollContext = {
    config: options.config ?? {},
    ...(options.since !== undefined && { since: options.since }),
    ...(options.cursor !== undefined && { cursor: options.cursor }),
    ...(options.limit !== undefined && { limit: options.limit }),
    now,
    // A poll only reads, so every failure it meets is worth trying again.
    fetch: resilientFetch({
      fetchImpl: options.fetchImpl ?? globalThis.fetch,
      retryable: true,
      ...(options.retry !== undefined && { retry: options.retry }),
      ...(options.sleep !== undefined && { sleep: options.sleep })
    })
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
  retry?: RetryPolicy
  /** Replaced in tests so backoff costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

/** Methods that change nothing, so repeating one cannot do a thing twice. */
const SAFE_METHODS = new Set(['GET', 'HEAD'])

function coerceArg(value: unknown, type: string | undefined): unknown {
  if (typeof value !== 'string') return value
  if (type === 'number') {
    const parsed = Number(value)
    if (Number.isNaN(parsed)) throw new Error(`Expected a number, got "${value}"`)
    return parsed
  }
  if (type === 'boolean') {
    if (value === 'true') return true
    if (value === 'false') return false
    throw new Error(`Expected a boolean, got "${value}"`)
  }
  return value
}

/**
 * Run an action with its declared inputs validated and coerced. Vorn renders
 * every action argument as a template string, so numbers and booleans arrive
 * as text and have to be converted back here.
 */
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
    if (value === undefined || value === '') {
      if (input.required) throw new Error(`Action ${actionType} requires "${input.key}"`)
      delete coerced[input.key]
      continue
    }
    try {
      coerced[input.key] = coerceArg(value, input.type)
    } catch (error) {
      throw new Error(
        `Action ${actionType} argument "${input.key}": ${
          error instanceof Error ? error.message : String(error)
        }`,
        { cause: error }
      )
    }
  }

  const config = options.config ?? {}
  // Repeating a write invents a second one, so a retry needs the action's word
  // that it is safe — except for a declared read, which says so by its method.
  const method = (action.request?.method ?? 'GET').toUpperCase()
  const retryable =
    action.idempotent === true || (action.request !== undefined && SAFE_METHODS.has(method))
  const fetchImpl = resilientFetch({
    fetchImpl: options.fetchImpl ?? globalThis.fetch,
    retryable,
    ...(options.retry !== undefined && { retry: options.retry }),
    ...(options.sleep !== undefined && { sleep: options.sleep })
  })

  if (typeof action.run === 'function') {
    const output = await action.run(coerced, {
      config,
      now: options.now ?? (() => new Date().toISOString()),
      fetch: fetchImpl
    })
    return output ?? {}
  }

  // Declared rather than written: `defineConnector` guarantees one or the other.
  return executeRequest(
    action.request,
    action.postReceive,
    { args: coerced, config },
    { fetchImpl }
  )
}
