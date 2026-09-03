import type {
  AuthRung,
  Connector,
  ConnectorConfig,
  ConnectorDefinition,
  DedupeStrategy
} from './types'

const KEY_PATTERN = /^[a-zA-Z][a-zA-Z0-9_-]*$/

/**
 * Characters that appear in SVG path data: the command letters, digits, and
 * the separators and exponent notation a number can use.
 *
 * Anything else is rejected here rather than at render time, because the app
 * drawing this icon has no way to tell a typo from an attempt to break out of
 * the `d` attribute.
 */
const PATH_DATA_PATTERN = /^[MmZzLlHhVvCcSsQqTtAa0-9\s,.\-+eE]+$/
const VIEW_BOX_PATTERN = /^-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+$/
const DEDUPE_STRATEGIES: DedupeStrategy[] = ['timestamp', 'lastItem']
const AUTH_RUNGS: AuthRung[] = ['none', 'cli', 'key', 'oauth']

/** A declared request goes somewhere the connector named: a real URL, … */
const ABSOLUTE_URL_PATTERN = /^https?:\/\//i
/** … or one built on a value from its own settings. */
const CONFIG_ROOTED_URL_PATTERN = /^\{\{\s*config\./

function assertUnique(kind: string, keys: string[]): void {
  const seen = new Set<string>()
  for (const key of keys) {
    if (seen.has(key)) throw new Error(`Duplicate ${kind} "${key}"`)
    seen.add(key)
  }
}

/**
 * Check that a declared auth block says enough for the host to act on it.
 *
 * Each rung promises the app something specific — that there is a command to
 * ask who you are, that a named field holds the credential, that there is
 * nothing to ask for at all. A rung whose promise is unbacked would be found
 * out at connection time, in front of someone trying to sign in.
 */
function assertAuth(definition: ConnectorDefinition): void {
  const auth = definition.auth
  if (!auth) return
  const id = definition.id

  if (!AUTH_RUNGS.includes(auth.rung)) {
    throw new Error(
      `Connector ${id} declares unknown auth rung ${JSON.stringify(auth.rung)}; ` +
        `expected ${AUTH_RUNGS.join(', ')}`
    )
  }

  if (auth.rung === 'cli' && !auth.probe?.command?.trim()) {
    throw new Error(`Connector ${id} borrows a CLI login but declares no probe command to ask it`)
  }

  if (auth.rung === 'key') {
    const keys = auth.keys ?? []
    if (keys.length === 0) {
      throw new Error(`Connector ${id} signs in with a key but names no config field holding it`)
    }
    const declared = new Set((definition.config ?? []).map((field) => field.key))
    for (const key of keys) {
      if (!declared.has(key)) {
        throw new Error(`Connector ${id} names auth key "${key}", which is not a config field`)
      }
    }
  }

  if (auth.rung === 'none') {
    const secret = (definition.config ?? []).find((field) => field.secret === true)
    if (secret) {
      throw new Error(
        `Connector ${id} claims it needs no sign-in but declares secret field "${secret.key}"`
      )
    }
  }
}

/** Environment variable a config field reads from, e.g. `apiToken` → `API_TOKEN`. */
export function envNameFor(key: string, explicit?: string): string {
  if (explicit) return explicit
  return key
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/[-\s]+/g, '_')
    .toUpperCase()
}

/**
 * Validate a connector definition and fill in its defaults.
 *
 * Failing here — at import time — is the whole point: a typo in a trigger
 * type or a duplicate action key otherwise surfaces as a silently missing
 * MCP tool once the connector is already installed in someone's app.
 */
export function defineConnector(definition: ConnectorDefinition): Connector {
  if (!KEY_PATTERN.test(definition.id ?? '')) {
    throw new Error(`Connector id "${definition.id}" must start with a letter and be url-safe`)
  }
  if (!definition.name?.trim()) {
    throw new Error(`Connector ${definition.id} is missing a name`)
  }

  if (definition.icon) {
    const { viewBox, paths } = definition.icon
    if (!Array.isArray(paths) || paths.length === 0) {
      throw new Error(`Connector ${definition.id} has an icon with no paths`)
    }
    for (const path of paths) {
      if (typeof path !== 'string' || !PATH_DATA_PATTERN.test(path)) {
        throw new Error(
          `Connector ${definition.id} has an icon path that is not SVG path data. ` +
            `Only path data is accepted, not markup.`
        )
      }
    }
    if (viewBox !== undefined && !VIEW_BOX_PATTERN.test(viewBox)) {
      throw new Error(`Connector ${definition.id} has an icon viewBox that is not four numbers`)
    }
  }

  const triggers = definition.triggers ?? []
  const actions = definition.actions ?? []
  if (triggers.length === 0 && actions.length === 0) {
    throw new Error(`Connector ${definition.id} declares no triggers and no actions`)
  }

  for (const trigger of triggers) {
    if (!KEY_PATTERN.test(trigger.type ?? '')) {
      throw new Error(`Trigger type "${trigger.type}" must start with a letter and be url-safe`)
    }
    // `TriggerDefinition` already rules these out for TypeScript authors; the
    // checks stay for plain-JS connectors, where the union buys nothing.
    const loose = trigger as { dedupe?: unknown; fetch?: unknown; poll?: unknown }
    const declarative = typeof loose.fetch === 'function'
    const imperative = typeof loose.poll === 'function'
    if (declarative && imperative) {
      throw new Error(`Trigger ${trigger.type} declares both fetch() and poll(); pick one`)
    }
    if (declarative !== (loose.dedupe !== undefined)) {
      throw new Error(
        `Trigger ${trigger.type} needs fetch() and a dedupe strategy together, not one alone`
      )
    }
    if (loose.dedupe !== undefined && !DEDUPE_STRATEGIES.includes(loose.dedupe as DedupeStrategy)) {
      // Without this a typo runs a strategy the author did not ask for, which
      // shows up as mis-delivered items rather than as an error.
      throw new Error(
        `Trigger ${trigger.type} has unknown dedupe strategy ${JSON.stringify(loose.dedupe)}; ` +
          `expected ${DEDUPE_STRATEGIES.join(' or ')}`
      )
    }
    if (loose.poll !== undefined && !imperative) {
      throw new Error(`Trigger ${trigger.type} declares poll but it is not a function`)
    }
    if (!declarative && !imperative) {
      throw new Error(`Trigger ${trigger.type} is missing a fetch() or poll() implementation`)
    }
  }
  for (const action of actions) {
    if (!KEY_PATTERN.test(action.type ?? '')) {
      throw new Error(`Action type "${action.type}" must start with a letter and be url-safe`)
    }
    // As with triggers, the union already rules these out for TypeScript
    // authors; the checks stay for plain-JS connectors, where it buys nothing.
    const loose = action as { run?: unknown; request?: unknown; postReceive?: unknown }
    const written = typeof loose.run === 'function'
    const declared = loose.request !== undefined
    if (written && declared) {
      throw new Error(`Action ${action.type} declares both run() and a request; pick one`)
    }
    if (!written && !declared) {
      throw new Error(`Action ${action.type} is missing a run() implementation or a request`)
    }
    if (declared) {
      const request = loose.request as { url?: unknown }
      if (typeof request?.url !== 'string' || request.url.trim() === '') {
        throw new Error(`Action ${action.type} declares a request with no URL`)
      }
      const url = request.url.trim()
      // Where the call goes has to be the connector's decision. A URL built
      // from an argument would let a step aim the connector's own credentials
      // at a host of its choosing, and a relative one names no host at all.
      if (!ABSOLUTE_URL_PATTERN.test(url) && !CONFIG_ROOTED_URL_PATTERN.test(url)) {
        throw new Error(
          `Action ${action.type} declares the request URL "${url}", which is neither absolute ` +
            `nor rooted in a {{config.…}} value`
        )
      }
    }
    if (!declared && loose.postReceive !== undefined) {
      throw new Error(`Action ${action.type} has postReceive but no request for it to reshape`)
    }
    for (const input of action.inputs ?? []) {
      // A field pointing at a set nobody serves draws an empty picker in the
      // app, which reads as "this connection has none" rather than as a typo.
      if (
        input.loadOptions !== undefined &&
        definition.options?.[input.loadOptions] === undefined
      ) {
        throw new Error(
          `Action ${action.type} argument "${input.key}" loads options from ` +
            `"${input.loadOptions}", which the connector does not serve`
        )
      }
    }
  }

  assertUnique(
    'trigger',
    triggers.map((trigger) => trigger.type)
  )
  assertUnique(
    'action',
    actions.map((action) => action.type)
  )
  assertUnique(
    'config field',
    (definition.config ?? []).map((field) => field.key)
  )
  assertAuth(definition)

  return {
    ...definition,
    version: definition.version ?? '0.0.0',
    config: definition.config ?? [],
    triggers,
    actions
  }
}

/**
 * Read the connector's declared config out of the environment. Vorn supplies
 * these through the connection's `env` / `secretEnv` maps, so a missing
 * required value is a setup mistake worth reporting by name rather than
 * letting the first API call fail with a confusing 401.
 */
export function resolveConfig(
  connector: Connector,
  env: NodeJS.ProcessEnv = process.env
): ConnectorConfig {
  const config: ConnectorConfig = {}
  const missing: string[] = []
  for (const field of connector.config) {
    const name = envNameFor(field.key, field.env)
    const value = env[name] ?? field.default
    if (value === undefined || value === '') {
      if (field.required) missing.push(`${field.key} (${name})`)
      continue
    }
    config[field.key] = value
  }
  if (missing.length > 0) {
    throw new Error(
      `Connector ${connector.id} is missing required configuration: ${missing.join(', ')}`
    )
  }
  return config
}
