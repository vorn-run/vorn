/**
 * Surviving an upstream's bad minute.
 *
 * Every connector eventually meets the same three answers — a rate limit, a
 * gateway that briefly forgot how to work, a socket that died mid-call — and
 * every author writes the same retry loop for them, usually without the one
 * part that matters: only repeating calls that are safe to repeat. Doing it
 * here means a connector gets it by saying nothing at all.
 */

/** Statuses worth trying again. Anything else is an answer, not a hiccup. */
const RETRYABLE_STATUS = new Set([408, 425, 429, 500, 502, 503, 504])

export interface RetryPolicy {
  /** Total tries, including the first. Defaults to 3. */
  attempts?: number
  /** First backoff step; each retry doubles it. Defaults to 250ms. */
  baseDelayMs?: number
  /** Ceiling for any single wait, including one the server asked for. */
  maxDelayMs?: number
}

export interface ResilientFetchOptions {
  fetchImpl: typeof fetch
  /**
   * Whether repeating the call is safe. A read always is; a write is only when
   * the action said so, because retrying a `create` invents a second one.
   */
  retryable: boolean
  retry?: RetryPolicy
  /** Replaced in tests so backoff costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

const DEFAULT_ATTEMPTS = 3
const DEFAULT_BASE_DELAY_MS = 250
const DEFAULT_MAX_DELAY_MS = 30_000

/**
 * The bounds a connector cannot talk its way past.
 *
 * A policy is the connector's to set, but a step that waits forever is nobody's
 * intention — an author who asks for fifty tries, or an upstream that keeps
 * asking for another minute, would otherwise hold a run open indefinitely.
 */
const MAX_ATTEMPTS = 10
const MAX_TOTAL_WAIT_MS = 120_000

const wait = (ms: number): Promise<void> =>
  new Promise((resolve) => {
    setTimeout(resolve, ms)
  })

/**
 * How long the server asked us to wait, in milliseconds.
 *
 * `Retry-After` is either a count of seconds or an HTTP date; both are common
 * enough that reading only one of them is how a connector ends up hammering a
 * rate limiter it was politely asked to back off from.
 */
export function retryAfterMs(header: string | null, now: number): number | undefined {
  if (!header) return undefined
  const trimmed = header.trim()
  if (trimmed === '') return undefined

  const seconds = Number(trimmed)
  if (Number.isFinite(seconds)) return Math.max(0, seconds * 1000)

  const at = Date.parse(trimmed)
  if (Number.isNaN(at)) return undefined
  return Math.max(0, at - now)
}

/** The wait before try number `attempt`, counting the first try as zero. */
export function backoffMs(attempt: number, policy: RetryPolicy = {}): number {
  const base = policy.baseDelayMs ?? DEFAULT_BASE_DELAY_MS
  const max = policy.maxDelayMs ?? DEFAULT_MAX_DELAY_MS
  // Deliberately without jitter: a connector's tests should be able to say
  // exactly how long it waited, and the fleet this serves is one machine's
  // worth of calls rather than a thundering herd.
  return Math.min(max, base * 2 ** attempt)
}

/**
 * Wrap a fetch so it retries what is worth retrying.
 *
 * The wrapper is the value handed to actions as `context.fetch`, so a
 * hand-written action and a declared request are equally protected.
 */
export function resilientFetch(options: ResilientFetchOptions): typeof fetch {
  const attempts = Math.min(MAX_ATTEMPTS, Math.max(1, options.retry?.attempts ?? DEFAULT_ATTEMPTS))
  const sleep = options.sleep ?? wait

  const ceiling = options.retry?.maxDelayMs ?? DEFAULT_MAX_DELAY_MS

  const send = async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    let waited = 0
    /** Wait, unless doing so would spend more than one call is allowed. */
    const pause = async (ms: number): Promise<boolean> => {
      if (waited + ms > MAX_TOTAL_WAIT_MS) return false
      waited += ms
      await sleep(ms)
      return true
    }

    for (let attempt = 0; attempt < attempts; attempt++) {
      const last = attempt === attempts - 1
      try {
        const response = await options.fetchImpl(input, init)
        if (!RETRYABLE_STATUS.has(response.status)) return response
        // Out of tries, or not ours to repeat: the status is the answer.
        if (!options.retryable || last) return response
        // The server named its own wait; honour it over our backoff, because
        // it knows when the limit resets and we are only guessing.
        const asked = retryAfterMs(response.headers.get('retry-after'), Date.now())
        const delay =
          asked === undefined ? backoffMs(attempt, options.retry) : Math.min(asked, ceiling)
        // Out of patience rather than out of tries; the status is still the answer.
        if (!(await pause(delay))) return response
      } catch (error) {
        // A thrown fetch is the network failing rather than the server
        // answering, which is exactly the case a retry exists for.
        if (!options.retryable || last) throw error
        if (!(await pause(backoffMs(attempt, options.retry)))) throw error
      }
    }
    // `attempts` is at least 1, so the loop always returns or throws first.
    throw new Error('Request was never attempted')
  }

  return send as unknown as typeof fetch
}
