import {
  bundleDependencyFindings,
  lifecycleScriptFindings,
  packEntryContents,
  readNearestPackageJson,
  type BundleOutput,
  type BundleRequest
} from './packaging'
import { withMockHttp, type MockRoute } from './harness'
import { runAction, runPoll, type PollPage } from './runtime'
import type {
  ActionDefinition,
  ActionInputField,
  Connector,
  ConnectorConfig,
  DedupeStrategy,
  TriggerDefinition
} from './types'

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
  /**
   * Directory whose nearest package.json says how the connector ships. Given,
   * the checks that are about the package rather than the definition run too.
   */
  packageDir?: string
  /**
   * Bundles the connector so the check can see what would stay outside it.
   * Given, a pack's no-install-step promise is verified before packing.
   */
  bundle?(request: BundleRequest): Promise<BundleOutput>
  /** Module specifier the bundle starts from; required by `bundle`. */
  entry?: string
  /**
   * Run every action against served HTTP rather than the network. Without
   * routes each request is answered `{}`, which proves an action runs and
   * escapes nowhere; with them, that it does the right thing.
   */
  mock?: boolean
  mockRoutes?: MockRoute[]
}

/** What the host will run a probe by, so a check refuses what it would drop. */
const EXECUTABLE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/

/** Config keys that name a credential, whatever the connector calls them. */
const CREDENTIAL_NAME = /(secret|token|password|passphrase|api[-_]?key|credential)/i

const INPUT_TYPES = new Set(['string', 'number', 'boolean', 'select', 'json'])

function finding(
  level: CheckFinding['level'],
  code: string,
  target: string,
  message: string
): CheckFinding {
  return { level, code, target, message }
}

/**
 * How the connector says it signs in, checked against what the host will keep.
 *
 * `defineConnector` already refuses a rung it cannot back up, so what is left
 * here is the gap between the two: an author-side probe that passes validation
 * and is then dropped at the host for not being a bare executable name, which
 * would silently turn a Sign in into a token field.
 */
function authFindings(connector: Connector): CheckFinding[] {
  const auth = connector.auth
  if (!auth) {
    return [
      finding(
        'warn',
        'auth-undeclared',
        connector.id,
        'does not say how it signs in, so the app cannot tell anyone before they install it'
      )
    ]
  }

  const found: CheckFinding[] = []
  const command = auth.probe?.command?.trim() ?? ''
  const args = auth.probe?.args ?? []
  if (auth.rung === 'cli') {
    if (!EXECUTABLE_NAME.test(command)) {
      found.push(
        finding(
          'error',
          'auth-probe-missing',
          `${connector.id} auth`,
          `probe command "${command}" is not a bare executable name, so the host drops it and the rung promises a sign-in it cannot ask for`
        )
      )
    }
    if (args.some((arg) => typeof arg !== 'string')) {
      found.push(
        finding(
          'error',
          'auth-probe-missing',
          `${connector.id} auth`,
          'probe arguments must all be strings, or the host drops the probe'
        )
      )
    }
  }

  return found
}

/** A credential Vorn would store in the clear because nothing marked it secret. */
function secretFindings(connector: Connector): CheckFinding[] {
  const named = new Set(connector.auth?.keys ?? [])
  return connector.config
    .filter((field) => !field.secret)
    .filter((field) => named.has(field.key) || CREDENTIAL_NAME.test(field.key))
    .map((field) =>
      finding(
        named.has(field.key) ? 'error' : 'warn',
        'secret-not-marked',
        `config ${field.key}`,
        'holds a credential but is not marked `secret`, so Vorn would store it unencrypted'
      )
    )
}

/** What an action takes and returns, as far as a step can see it before running. */
function actionShapeFindings(action: ActionDefinition): CheckFinding[] {
  const target = `action ${action.type}`
  const found: CheckFinding[] = []

  if (!action.outputs?.length) {
    found.push(
      finding(
        'warn',
        'action-no-outputs',
        target,
        'declares no outputs, so a later step has nothing to autocomplete from'
      )
    )
  }

  for (const input of action.inputs ?? []) {
    if (input.type !== undefined && !INPUT_TYPES.has(input.type)) {
      found.push(
        finding(
          'error',
          'input-type-unsupported',
          `${target} input ${input.key}`,
          `declares type "${input.type}", which Vorn cannot draw a field for`
        )
      )
    }
    // A select is a promise of choices; one with neither list is a text box
    // wearing a dropdown's clothes.
    if (input.type === 'select' && !input.options?.length && !input.loadOptions) {
      found.push(
        finding(
          'error',
          'input-type-unsupported',
          `${target} input ${input.key}`,
          'is a select with neither fixed options nor a loadOptions set to draw from'
        )
      )
    }
  }

  return found
}

/** What the package says about itself, when a check was pointed at one. */
async function packageFindings(options: CheckOptions): Promise<CheckFinding[]> {
  if (options.packageDir === undefined) return []
  const pkg = readNearestPackageJson(options.packageDir)
  const found = [...lifecycleScriptFindings(pkg)]

  const vorn = (pkg as { vorn?: { keywords?: unknown } } | undefined)?.vorn
  const keywords = Array.isArray(vorn?.keywords) ? vorn.keywords : []
  if (keywords.length === 0) {
    found.push(
      finding(
        'warn',
        'keywords-missing',
        'package.json',
        'names no keywords, so the connector is findable only by its own name'
      )
    )
  }

  if (options.bundle && options.entry !== undefined) {
    const built = await options.bundle({
      contents: packEntryContents(options.entry),
      resolveDir: options.packageDir
    })
    found.push(...bundleDependencyFindings(built.external))
  }

  return found
}

/** A value of the declared type, so an action can be run without a person. */
function sampleArg(input: ActionInputField): string {
  if (input.type === 'number') return '1'
  if (input.type === 'boolean') return 'false'
  if (input.type === 'json') return '{}'
  if (input.type === 'select') return input.options?.[0]?.value ?? 'check'
  return 'check'
}

/**
 * Run every action once with nothing but served HTTP behind it.
 *
 * Two things are being asked. That an action runs at all on its own declared
 * arguments — until now no check ever called one — and that it reaches nothing
 * the routes did not offer, which is what makes a conformance run hermetic.
 *
 * A failure is an error only when the caller supplied routes: they said what
 * the service returns, so a throw is the connector's. Against the bare `{}`
 * default it is a warning, because an empty object is not a real reply.
 */
async function mockFindings(connector: Connector, options: CheckOptions): Promise<CheckFinding[]> {
  if (!options.mock) return []
  const routes = options.mockRoutes ?? [{ url: /.*/ }]
  const level = options.mockRoutes?.length ? 'error' : 'warn'
  const found: CheckFinding[] = []

  for (const action of connector.actions) {
    const args = Object.fromEntries(
      (action.inputs ?? []).map((input) => [input.key, sampleArg(input)])
    )
    try {
      await withMockHttp(routes, () =>
        runAction(connector, action.type, args, {
          config: options.config ?? {},
          ...(options.now && { now: options.now })
        })
      )
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      // Reaching for the network is a failure in either mode: a conformance
      // run that touches a real service is not a conformance run.
      const escaped = reason.startsWith('No mock route')
      found.push(
        finding(
          escaped ? 'error' : level,
          escaped ? 'mock-network-escape' : 'mock-action-failed',
          `action ${action.type}`,
          `did not run against served HTTP: ${reason}`
        )
      )
    }
  }

  return found
}

/**
 * Ask the connector, against the real service, the questions only it can answer.
 *
 * Preflight first, because a connector that cannot sign in fails every later
 * check for one uninteresting reason. Then each action that declared itself
 * idempotent — and only those: a live run of `createIssue` would leave real
 * issues behind, so a smoke test never calls one.
 */
async function liveFindings(connector: Connector, options: CheckOptions): Promise<CheckFinding[]> {
  if (!options.live) return []
  const found: CheckFinding[] = []

  if (connector.preflight) {
    try {
      const result = await connector.preflight()
      if (!result.ok) {
        found.push(
          finding(
            'error',
            'preflight-failed',
            connector.id,
            result.message ?? 'reported that it is not ready, without saying why'
          )
        )
        // Nothing below can succeed if it cannot sign in, and each failure
        // would repeat this one in a less useful sentence.
        return found
      }
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      found.push(finding('error', 'preflight-failed', connector.id, `threw: ${reason}`))
      return found
    }
  }

  for (const action of connector.actions.filter((entry) => entry.idempotent === true)) {
    const args = Object.fromEntries(
      (action.inputs ?? []).map((input) => [input.key, sampleArg(input)])
    )
    try {
      await runAction(connector, action.type, args, {
        config: options.config ?? {},
        ...(options.now && { now: options.now })
      })
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      found.push(
        finding('error', 'live-action-failed', `action ${action.type}`, `threw: ${reason}`)
      )
    }
  }

  return found
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

  found.push(...authFindings(connector))
  found.push(...secretFindings(connector))
  found.push(...(await packageFindings(options)))
  found.push(...(await mockFindings(connector, options)))
  found.push(...(await liveFindings(connector, options)))

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
    found.push(...actionShapeFindings(action))
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

/**
 * What the factory checked, and when.
 *
 * Mirrors the receipt the catalog carries. "Verified" is not a word here but a
 * list: the checks that ran and came back with nothing to say. A check that
 * could not run — no sample to replay, no credentials to go live with — is
 * absent rather than passed, because absent is the true answer.
 */
export interface ConnectorVerification {
  /** Which receipt format this is, so a later one is not read as this one. */
  schema: 1
  version: string
  checkedAt: string
  checks: string[]
}

/** Which named check each finding belongs to, so one failure clears one name. */
const CHECK_OWNERS: Record<string, string> = {
  'missing-description': 'manifest',
  'auth-undeclared': 'auth',
  'auth-probe-missing': 'auth',
  'secret-not-marked': 'secrets',
  'action-no-outputs': 'actions',
  'input-type-unsupported': 'actions',
  'missing-idempotent': 'actions',
  'poll-failed': 'dedupe',
  'no-items': 'dedupe',
  'no-cursor': 'dedupe',
  'cursor-rejected': 'dedupe',
  'redelivers-items': 'dedupe',
  'stuck-cursor': 'dedupe',
  unverifiable: 'dedupe',
  'sample-unusable': 'dedupe',
  'lifecycle-scripts': 'no-lifecycle-scripts',
  'keywords-missing': 'keywords',
  'runtime-dependencies': 'no-runtime-deps',
  'mock-action-failed': 'mock',
  'mock-network-escape': 'mock',
  'preflight-failed': 'live',
  'live-action-failed': 'live'
}

/** The checks a run of these options actually performs. */
function checksRun(options: CheckOptions): string[] {
  const names = ['manifest', 'auth', 'secrets', 'actions', 'dedupe']
  if (options.packageDir !== undefined) names.push('no-lifecycle-scripts', 'keywords')
  if (options.bundle && options.entry !== undefined) names.push('no-runtime-deps')
  if (options.mock) names.push('mock')
  if (options.live) names.push('live')
  return names
}

export interface ConformanceRun {
  findings: CheckFinding[]
  /** Named checks that ran and had nothing to say. */
  passed: string[]
  /**
   * The receipt to publish, or nothing when an error means there is no claim
   * to make. Warnings do not void it — they are advice, not a failure.
   */
  receipt?: ConnectorVerification
}

/**
 * Check a connector and say what can be vouched for.
 *
 * `checkConnector` answers "what is wrong"; this answers the catalog's
 * question, "what did you check", which is what a verified badge shows.
 */
export async function runConformance(
  connector: Connector,
  options: CheckOptions = {}
): Promise<ConformanceRun> {
  const findings = await checkConnector(connector, options)
  const spoiled = new Set(findings.map((item) => CHECK_OWNERS[item.code]).filter(Boolean))
  const passed = checksRun(options).filter((name) => !spoiled.has(name))
  const failed = findings.some((item) => item.level === 'error')
  const now = options.now ?? (() => new Date().toISOString())

  return {
    findings,
    passed,
    ...(!failed && {
      receipt: {
        schema: 1 as const,
        version: connector.version,
        checkedAt: now(),
        checks: passed
      }
    })
  }
}

/** Render findings for a terminal. Returns an empty string when all clear. */
export function formatFindings(findings: CheckFinding[]): string {
  return findings
    .map((item) => `${item.level.padEnd(5)}  ${item.target}: ${item.message} [${item.code}]`)
    .join('\n')
}
