import type {
  ActivationPredicate,
  AuthRung,
  BrowserSignIn,
  Connector,
  ConnectorConfig,
  ConnectorDefinition,
  ConnectorIcon,
  DedupeStrategy,
  ExtensionAgent,
  ExtensionDefinition,
  ExtensionHostMethod,
  ExtensionPermission,
  ExtensionPlatform,
  PaneContribution
} from './types'
import { ORIGIN_PATTERN, allowedSessionHeader, withinOrigins } from './origins'
import { isRecord } from './post-receive'

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
const AUTH_RUNGS: AuthRung[] = ['none', 'cli', 'key', 'browser', 'oauth']

/** Everything an extension may ask the host for; anything else is not grantable. */
export const EXTENSION_PERMISSIONS: ExtensionPermission[] = [
  'git.read',
  'terminal.read',
  'terminal.selection',
  'terminal.send',
  'card.rename',
  'agent.usage'
]

/** What each host method costs, read by the bridge that grants it and the check that gates it. */
export const HOST_PERMISSIONS: Record<ExtensionHostMethod, ExtensionPermission> = {
  diff: 'git.read',
  status: 'git.read',
  output: 'terminal.read',
  selection: 'terminal.selection',
  send: 'terminal.send',
  rename: 'card.rename',
  usage: 'agent.usage'
}

/** Session types an extension may name, so a manifest cannot invent one. */
export const EXTENSION_AGENTS: ExtensionAgent[] = [
  'claude',
  'copilot',
  'codex',
  'opencode',
  'gemini',
  'shell'
]
export const EXTENSION_PLATFORMS: ExtensionPlatform[] = ['darwin', 'linux', 'win32']

/** A pane's page lives under `web/` in the package, so a pack carries one named directory. */
const WEB_ENTRY_PATTERN = /^web\/[A-Za-z0-9._/-]+\.html$/

/** Slower than this and a footer is a poller; faster and it is a spinner. */
const MIN_FOOTER_SECONDS = 5

/** A pattern is matched against clicked text on a person's keystroke, so it stays small enough to bound. */
const MAX_PATTERN_LENGTH = 256

/** How a group opens, including the forms that are not a capture. */
const GROUP_OPEN = /^\((\?(:|=|!|<=|<!|<[A-Za-z_$][\w$]*>))?/

/**
 * Whether a quantifier is applied to a group that already holds one.
 *
 * `(a+)+` and its shapes are what make a match cost exponential time on text
 * that nearly fits, and a link pattern is matched on a click. Refused where the
 * author can see why rather than bounded where a person would only feel it.
 *
 * Deliberately shape-based: it asks whether the pattern is written that way, not
 * whether that particular pattern is slow, so it refuses a little more than it
 * must and never less.
 */
export function hasNestedQuantifier(pattern: string): boolean {
  // An index past the end is not a quantifier; `includes('')` would say it is.
  const quantifierAt = (at: number): boolean => {
    const ch = pattern[at]
    return ch !== undefined && '*+?{'.includes(ch)
  }
  // One flag per open group: whether anything inside it is quantified.
  const quantified: boolean[] = []
  let inClass = false

  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i]
    if (ch === '\\') {
      i++
      continue
    }
    if (inClass) {
      if (ch === ']') inClass = false
      continue
    }
    if (ch === '[') {
      inClass = true
      continue
    }
    if (ch === '(') {
      quantified.push(false)
      i += (GROUP_OPEN.exec(pattern.slice(i))?.[0].length ?? 1) - 1
      continue
    }
    if (ch === ')') {
      const heldOne = quantified.pop() ?? false
      const repeated = quantifierAt(i + 1)
      if (heldOne && repeated) return true
      // Whatever this group holds or does, its parent holds a quantifier too.
      if ((heldOne || repeated) && quantified.length > 0) {
        quantified[quantified.length - 1] = true
      }
      continue
    }
    if (quantifierAt(i) && quantified.length > 0) quantified[quantified.length - 1] = true
  }
  return false
}

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

  if (auth.rung === 'browser') assertBrowserSignIn(id, auth.browser)

  if (auth.rung === 'none' || auth.rung === 'browser') {
    const secret = (definition.config ?? []).find((field) => field.secret === true)
    if (secret) {
      const claim = auth.rung === 'none' ? 'needs no sign-in' : 'signs in through a Vorn window'
      throw new Error(
        `Connector ${id} claims it ${claim} but declares secret field "${secret.key}"`
      )
    }
  }
}

/** The window, the origins it may act on, and the check that says who is signed in. */
function assertBrowserSignIn(id: string, browser: BrowserSignIn | undefined): void {
  if (!browser) {
    throw new Error(
      `Connector ${id} signs in through a Vorn window but declares no browser sign-in`
    )
  }
  const origins = Array.isArray(browser.origins) ? browser.origins : []
  const bad = origins.find((origin) => typeof origin !== 'string' || !ORIGIN_PATTERN.test(origin))
  if (origins.length === 0 || bad !== undefined) {
    throw new Error(
      `Connector ${id} must name its origins as https://host or https://*.host` +
        (bad !== undefined ? `; ${JSON.stringify(bad)} is neither` : '')
    )
  }
  const places: Array<[string, unknown]> = [
    ['sign-in page', browser.signInUrl],
    ['signed-in check', browser.check?.url]
  ]
  for (const [what, url] of places) {
    if (typeof url !== 'string' || !withinOrigins(origins, url)) {
      throw new Error(
        `Connector ${id} puts its ${what} ${JSON.stringify(url ?? '')} outside its origins`
      )
    }
  }
  const identity = browser.check?.identity
  if (
    !Array.isArray(identity) ||
    identity.some((path) => typeof path !== 'string' || !path.trim())
  ) {
    throw new Error(`Connector ${id} must name its identity fields as non-empty strings`)
  }
  const headers: unknown = browser.check?.headers
  if (headers === undefined) return
  const badHeader = isRecord(headers)
    ? Object.entries(headers).find(
        ([name, value]) => typeof value !== 'string' || !allowedSessionHeader(name)
      )
    : undefined
  if (!isRecord(headers) || badHeader) {
    throw new Error(
      `Connector ${id} may add only plain string headers to its signed-in check` +
        (badHeader ? `; ${JSON.stringify(badHeader[0])} is not one` : '')
    )
  }
}

/** The id, name and glyph every pack declares, whichever kind it is. */
function assertIdentity(
  kind: string,
  definition: { id?: string; name?: string; icon?: ConnectorDefinition['icon'] }
): void {
  if (!KEY_PATTERN.test(definition.id ?? '')) {
    throw new Error(`${kind} id "${definition.id}" must start with a letter and be url-safe`)
  }
  if (!definition.name?.trim()) {
    throw new Error(`${kind} ${definition.id} is missing a name`)
  }
  assertIcon(`${kind} ${definition.id}`, definition.icon)
}

/** Path data and nothing else, so a glyph cannot carry markup into the app drawing it. */
function assertIcon(subject: string, icon: ConnectorIcon | undefined): void {
  if (!icon) return

  const { viewBox, paths } = icon
  if (!Array.isArray(paths) || paths.length === 0) {
    throw new Error(`${subject} has an icon with no paths`)
  }
  for (const path of paths) {
    if (typeof path !== 'string' || !PATH_DATA_PATTERN.test(path)) {
      throw new Error(
        `${subject} has an icon path that is not SVG path data. ` +
          `Only path data is accepted, not markup.`
      )
    }
  }
  if (viewBox !== undefined && !VIEW_BOX_PATTERN.test(viewBox)) {
    throw new Error(`${subject} has an icon viewBox that is not four numbers`)
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
  assertIdentity('Connector', definition)

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
    kind: 'connector',
    version: definition.version ?? '0.0.0',
    config: definition.config ?? [],
    triggers,
    actions
  }
}

/** Each declared value has to be one this build knows, or the predicate silently never matches. */
function assertPredicate(id: string, where: string, predicate?: ActivationPredicate): void {
  if (!predicate) return
  const lists: Array<[string, unknown]> = [
    ['workspaceContains', predicate.workspaceContains],
    ['remoteHost', predicate.remoteHost],
    ['agent', predicate.agent],
    ['platform', predicate.platform]
  ]
  for (const [field, value] of lists) {
    if (value === undefined) continue
    if (!Array.isArray(value) || value.length === 0) {
      throw new Error(`Extension ${id} ${where} declares "${field}" with nothing in it`)
    }
    for (const entry of value) {
      if (typeof entry !== 'string' || entry.trim() === '') {
        throw new Error(`Extension ${id} ${where} declares an empty "${field}" value`)
      }
    }
  }
  for (const glob of predicate.workspaceContains ?? []) {
    // Resolved against the session's worktree, so a path leaving it names a file no session owns.
    if (glob.startsWith('/') || glob.split('/').includes('..')) {
      throw new Error(
        `Extension ${id} ${where} looks for "${glob}", which is not inside the worktree`
      )
    }
  }
  for (const agent of predicate.agent ?? []) {
    if (!EXTENSION_AGENTS.includes(agent)) {
      throw new Error(
        `Extension ${id} ${where} names unknown agent ${JSON.stringify(agent)}; ` +
          `expected ${EXTENSION_AGENTS.join(', ')}`
      )
    }
  }
  for (const platform of predicate.platform ?? []) {
    if (!EXTENSION_PLATFORMS.includes(platform)) {
      throw new Error(
        `Extension ${id} ${where} names unknown platform ${JSON.stringify(platform)}; ` +
          `expected ${EXTENSION_PLATFORMS.join(', ')}`
      )
    }
  }
}

/** A pane is a page the pack carries or a program it runs, and the two are told apart here. */
function assertPane(id: string, pane: PaneContribution): void {
  assertIcon(`Extension ${id} pane ${pane.id}`, pane.icon)
  const loose = pane as { web?: unknown; command?: unknown }
  const page = loose.web !== undefined
  const program = loose.command !== undefined
  if (page && program) {
    throw new Error(`Extension ${id} pane ${pane.id} declares both a web page and a command`)
  }
  if (!page && !program) {
    throw new Error(`Extension ${id} pane ${pane.id} declares neither a web page nor a command`)
  }
  if (page) {
    const web = loose.web
    if (typeof web !== 'string' || !WEB_ENTRY_PATTERN.test(web) || web.split('/').includes('..')) {
      throw new Error(
        `Extension ${id} pane ${pane.id} declares the page ${JSON.stringify(web)}; ` +
          `a page is an .html file under web/ in the package`
      )
    }
    return
  }
  const command = loose.command
  if (!Array.isArray(command) || command.length === 0) {
    throw new Error(`Extension ${id} pane ${pane.id} declares a command with nothing to run`)
  }
  for (const arg of command) {
    if (typeof arg !== 'string' || arg === '') {
      throw new Error(`Extension ${id} pane ${pane.id} declares a command with an empty argument`)
    }
  }
}

/**
 * Validate an extension and fill in its defaults.
 *
 * An extension is a pack like a connector, so it goes through the same
 * manifest, pack, check and catalog; what differs is that it contributes to a
 * session card rather than polling a service. Failing here — at import time —
 * keeps a mistyped permission or an unreachable page from reaching a card.
 */
export function defineExtension(definition: ExtensionDefinition): Connector {
  assertIdentity('Extension', definition)
  const id = definition.id

  const panes = definition.panes ?? []
  const footers = definition.footers ?? []
  const linkHandlers = definition.linkHandlers ?? []
  if (panes.length === 0 && footers.length === 0 && linkHandlers.length === 0) {
    throw new Error(`Extension ${id} contributes nothing`)
  }

  const permissions = definition.permissions ?? []
  if (!Array.isArray(permissions)) {
    throw new Error(`Extension ${id} declares permissions that are not a list`)
  }
  for (const permission of permissions) {
    if (!EXTENSION_PERMISSIONS.includes(permission)) {
      throw new Error(
        `Extension ${id} asks for unknown permission ${JSON.stringify(permission)}; ` +
          `expected ${EXTENSION_PERMISSIONS.join(', ')}`
      )
    }
  }
  assertUnique('permission', permissions)

  const contributions = [...panes, ...footers, ...linkHandlers]
  for (const contribution of contributions) {
    if (!KEY_PATTERN.test(contribution.id ?? '')) {
      throw new Error(
        `Contribution id "${contribution.id}" must start with a letter and be url-safe`
      )
    }
    if (!contribution.title?.trim()) {
      throw new Error(`Extension ${id} contribution ${contribution.id} is missing a title`)
    }
    assertPredicate(id, `contribution ${contribution.id}`, contribution.when)
  }
  // One namespace: a pane and a footer sharing an id would be two things the host addresses alike.
  assertUnique(
    'contribution',
    contributions.map((contribution) => contribution.id)
  )
  assertPredicate(id, 'activates', definition.activates)

  for (const pane of panes) assertPane(id, pane)

  for (const footer of footers) {
    if (typeof footer.run !== 'function') {
      throw new Error(`Extension ${id} footer ${footer.id} is missing a run() implementation`)
    }
    if (!Number.isFinite(footer.every) || footer.every < MIN_FOOTER_SECONDS) {
      throw new Error(
        `Extension ${id} footer ${footer.id} asks to run every ${footer.every}s; ` +
          `${MIN_FOOTER_SECONDS}s is the shortest interval a footer may ask for`
      )
    }
  }

  for (const handler of linkHandlers) {
    if (typeof handler.run !== 'function') {
      throw new Error(
        `Extension ${id} link handler ${handler.id} is missing a run() implementation`
      )
    }
    if (typeof handler.pattern !== 'string' || handler.pattern.length > MAX_PATTERN_LENGTH) {
      throw new Error(
        `Extension ${id} link handler ${handler.id} has a pattern longer than ` +
          `${MAX_PATTERN_LENGTH} characters; it is matched on every click`
      )
    }
    if (hasNestedQuantifier(handler.pattern)) {
      throw new Error(
        `Extension ${id} link handler ${handler.id} has a pattern that repeats a group which ` +
          `already repeats; matching it can take exponential time on a click`
      )
    }
    let matcher: RegExp
    try {
      matcher = new RegExp(handler.pattern)
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      throw new Error(
        `Extension ${id} link handler ${handler.id} has a pattern that is not a regular expression: ${reason}`,
        { cause: error }
      )
    }
    // The example is what `check` runs the handler on, so a pattern it does not
    // match would prove the handler against a link it will never be offered for.
    if (typeof handler.example !== 'string' || handler.example.trim() === '') {
      throw new Error(
        `Extension ${id} link handler ${handler.id} names no example link its pattern matches`
      )
    }
    if (!matcher.test(handler.example)) {
      throw new Error(
        `Extension ${id} link handler ${handler.id} has the example ${JSON.stringify(handler.example)}, ` +
          `which its own pattern ${JSON.stringify(handler.pattern)} does not match`
      )
    }
  }

  return {
    id,
    name: definition.name,
    ...(definition.description !== undefined && { description: definition.description }),
    ...(definition.icon !== undefined && { icon: definition.icon }),
    kind: 'extension',
    version: definition.version ?? '0.0.0',
    // Its credential is the host's own token, so there is nothing to sign in to.
    auth: { rung: 'none' },
    config: [],
    triggers: [],
    actions: [],
    permissions,
    ...(definition.activates !== undefined && { activates: definition.activates }),
    contributes: {
      ...(panes.length > 0 && { panes }),
      ...(footers.length > 0 && { footers }),
      ...(linkHandlers.length > 0 && { linkHandlers })
    }
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
