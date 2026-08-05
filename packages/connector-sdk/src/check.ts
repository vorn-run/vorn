import { runPoll, type PollPage } from './runtime'
import type { Connector, ConnectorConfig, DedupeStrategy, TriggerDefinition } from './types'

export interface CheckFinding {
  /** `error` means the connector will misbehave in Vorn; `warn` is advisory. */
  level: 'error' | 'warn'
  code: string
  /** Which part of the connector the finding is about. */
  target: string
  message: string
}

export interface CheckOptions {
  /**
   * Poll every trigger against the real source. Off by default, so a check
   * runs on declared `sample` items and the definition alone.
   */
  live?: boolean
  /** Credentials, required by `live`. */
  config?: ConnectorConfig
  now?: () => string
}

function finding(
  level: CheckFinding['level'],
  code: string,
  target: string,
  message: string
): CheckFinding {
  return { level, code, target, message }
}

/**
 * Replay a trigger's declared `sample` through the real dedupe pipeline by
 * swapping in a fetch that serves the whole sample on every call. Serving it
 * again on the second poll is the point: a correct trigger recognizes its own
 * cursor and delivers nothing, while one that redelivers is caught. Everything
 * else — cursor encoding, ordering, normalization — is the production path.
 */
function sampleTrigger(trigger: TriggerDefinition & { dedupe: DedupeStrategy }): TriggerDefinition {
  return { ...trigger, poll: undefined, fetch: () => trigger.sample ?? [] }
}

async function checkPollBehaviour(
  connector: Connector,
  trigger: TriggerDefinition,
  options: CheckOptions
): Promise<CheckFinding[]> {
  const found: CheckFinding[] = []
  const probe: Connector = { ...connector, triggers: [trigger] }
  const target = `trigger ${trigger.type}`
  const now = options.now ?? (() => new Date().toISOString())

  const attempt = async (
    cursor: string | undefined,
    code: string,
    what: string
  ): Promise<PollPage | CheckFinding> => {
    try {
      return await runPoll(probe, trigger.type, {
        config: options.config ?? {},
        ...(cursor !== undefined && { cursor }),
        now
      })
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      return finding('error', code, target, `${what} threw: ${reason}`)
    }
  }

  const first = await attempt(undefined, 'poll-failed', 'first poll')
  if ('level' in first) return [first]

  if (first.items.length === 0) {
    found.push(
      finding('warn', 'no-items', target, 'returned nothing, so delivery could not be verified')
    )
    return found
  }
  if (first.nextCursor === undefined) {
    found.push(
      finding(
        'error',
        'no-cursor',
        target,
        'returned items but no nextCursor, so every poll will redeliver them'
      )
    )
    return found
  }

  // The check that matters: a second poll carrying the returned cursor must
  // not hand back anything it already delivered.
  const second = await attempt(
    first.nextCursor,
    'cursor-rejected',
    're-polling with its own nextCursor'
  )
  if ('level' in second) return [...found, second]

  const delivered = new Set(first.items.map((item) => item.externalId))
  const repeated = second.items.filter((item) => delivered.has(item.externalId))
  if (repeated.length > 0) {
    found.push(
      finding(
        'error',
        'redelivers-items',
        target,
        `re-polling with its own nextCursor returned ${repeated.length} already-delivered item(s), starting with "${repeated[0]!.externalId}"`
      )
    )
  }
  if (second.hasMore && second.nextCursor === first.nextCursor) {
    found.push(
      finding('error', 'stuck-cursor', target, 'reports more pages but its cursor never advances')
    )
  }

  return found
}

/**
 * Check a connector against the contract Vorn relies on.
 *
 * The point is a feedback loop: a connector — hand-written or generated — can
 * be verified before it is ever installed, catching the failures that are
 * otherwise invisible until duplicate tasks show up in someone's inbox days
 * later.
 */
export async function checkConnector(
  connector: Connector,
  options: CheckOptions = {}
): Promise<CheckFinding[]> {
  const found: CheckFinding[] = []

  if (!connector.description?.trim()) {
    found.push(
      finding(
        'warn',
        'missing-description',
        connector.id,
        'has no description; agents use it to decide when the connector applies'
      )
    )
  }

  // Triggers share no state, so their checks run concurrently rather than
  // waiting on each other's polls.
  const perTrigger = await Promise.all(
    connector.triggers.map(async (trigger) => {
      const target = `trigger ${trigger.type}`
      const triggerFindings: CheckFinding[] = []
      if (!trigger.description?.trim()) {
        triggerFindings.push(finding('warn', 'missing-description', target, 'has no description'))
      }

      if (options.live) {
        triggerFindings.push(...(await checkPollBehaviour(connector, trigger, options)))
      } else if (!trigger.sample?.length) {
        triggerFindings.push(
          finding(
            'warn',
            'unverifiable',
            target,
            'has no sample items and no credentials were supplied, so nothing could be verified'
          )
        )
      } else if (!trigger.dedupe) {
        triggerFindings.push(
          finding(
            'warn',
            'sample-unusable',
            target,
            'declares sample items but implements poll() directly, so they cannot be replayed; re-run with --live'
          )
        )
      } else {
        triggerFindings.push(
          ...(await checkPollBehaviour(connector, sampleTrigger(trigger), options))
        )
      }
      return triggerFindings
    })
  )
  found.push(...perTrigger.flat())

  for (const action of connector.actions) {
    const target = `action ${action.type}`
    if (!action.description?.trim()) {
      found.push(
        finding('warn', 'missing-description', target, 'has no description for the agent to read')
      )
    }
    if (action.idempotent === undefined) {
      found.push(
        finding(
          'warn',
          'missing-idempotent',
          target,
          'does not declare `idempotent`, so an agent cannot tell whether retrying is safe'
        )
      )
    }
    for (const input of action.inputs ?? []) {
      if (!input.description?.trim()) {
        found.push(
          finding(
            'warn',
            'missing-description',
            `${target} input ${input.key}`,
            'has no description'
          )
        )
      }
    }
  }

  return found
}

/** Render findings for a terminal. Returns an empty string when all clear. */
export function formatFindings(findings: CheckFinding[]): string {
  return findings
    .map((item) => `${item.level.padEnd(5)}  ${item.target}: ${item.message} [${item.code}]`)
    .join('\n')
}
