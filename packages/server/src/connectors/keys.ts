/**
 * The secrets this machine holds, read as one list.
 *
 * A key is not a record of its own: it is a password-typed field on some
 * connection, and one value often serves several workflows. Reading them all
 * through the same rule — the connector's own manifest says which of its
 * fields are secret — means an HTTP profile's token, a packaged connector's
 * env blob and a built-in's API key are the same kind of thing here, and
 * rotating any of them takes one path instead of three.
 */
import type {
  ConnectorConfigField,
  ConnectorKey,
  ConnectorKeyField,
  SourceConnection,
  WorkflowDefinition,
  WorkflowNode
} from '@vornrun/shared/types'
import { connectionConnectorId } from '@vornrun/shared/types'
import { boundConnectionKey } from '@vornrun/shared/workflow-portability'

/** The one field that carries a set of env vars rather than a single value. */
export const SECRET_ENV_FIELD = 'secretEnv'

/** Which of a connector's fields hold secrets, as the connector itself says. */
export function passwordFields(auth: ConnectorConfigField[] | undefined): ConnectorConfigField[] {
  return (auth ?? []).filter((field) => field.type === 'password')
}

// Published key prefixes: naming one says which service a value belongs to without giving up the secret.
const VENDOR_MARKERS = [
  'sk_live_',
  'sk_test_',
  'pk_live_',
  'pk_test_',
  'rk_live_',
  'whsec_',
  'github_pat_',
  'ghp_',
  'gho_',
  'ghs_',
  'ghu_',
  'glpat-',
  'xoxb-',
  'xoxp-',
  'xoxa-',
  'xapp-',
  'shpat_',
  'npm_',
  'dop_v1_',
  'AKIA',
  'ASIA'
]

// A known marker plus the last four characters, the way a card is quoted; the middle is never shown.
export function maskSecret(value: string | undefined): string {
  if (!value) return ''
  // Too short for a tail to be a hint rather than most of the value.
  if (value.length < 12) return '••••'
  const marker = VENDOR_MARKERS.find((m) => value.startsWith(m)) ?? ''
  return `${marker}••••${value.slice(-4)}`
}

const RESERVED_NAMES = new Set(['__proto__', 'constructor', 'prototype'])

// A name a shell accepts and the prototype cannot be reached through.
export function isEnvName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) && !RESERVED_NAMES.has(name)
}

/** The env names a packaged connector's blob carries, when it can be read. */
export function envNamesOf(blob: string | undefined): string[] {
  if (!blob) return []
  try {
    const parsed: unknown = JSON.parse(blob)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return []
    return Object.keys(parsed as Record<string, unknown>).filter(isEnvName)
  } catch {
    return []
  }
}

/**
 * Every connection a step names, whether it runs against it or borrows from it.
 *
 * `boundConnectionKey` answers a narrower question — which connection a step
 * cannot run without — and a script's `secretsFrom` is deliberately not that:
 * it is optional, so a script missing one is not an unmet requirement. It is
 * still a real use of the key, and this page is asked what rotating one would
 * touch, so it is counted here rather than widened there.
 */
function boundConnectionIds(node: WorkflowNode, config: Record<string, unknown>): string[] {
  const ids: string[] = []
  const required = boundConnectionKey(node, config)
  if (required !== null) ids.push(String(config[required] ?? ''))
  if (node.type === 'script') ids.push(String(config.secretsFrom ?? ''))
  return ids.filter((id) => id !== '')
}

/**
 * How many workflow steps run against a connection.
 *
 * Counted per step rather than per workflow: "used by 3 steps" is what tells
 * someone what rotating this key is about to touch.
 */
export function usageCounts(workflows: WorkflowDefinition[]): Map<string, number> {
  const counts = new Map<string, number>()
  for (const workflow of workflows) {
    for (const node of workflow.nodes) {
      const config = (node.config ?? {}) as Record<string, unknown>
      for (const id of boundConnectionIds(node, config)) {
        counts.set(id, (counts.get(id) ?? 0) + 1)
      }
    }
  }
  return counts
}

/** What a stored field can say about itself, given what this build can read. */
function describeField(
  field: ConnectorConfigField,
  stored: unknown,
  decrypted: string | undefined
): ConnectorKeyField | undefined {
  // Nothing stored is not a key: an optional password nobody filled in has
  // nothing to test and nothing to rotate.
  if (typeof stored !== 'string' || stored === '') return undefined
  const readable = decrypted !== undefined
  if (field.key === SECRET_ENV_FIELD) {
    const names = envNamesOf(decrypted)
    return { key: field.key, label: field.label, readable, envNames: names }
  }
  return { key: field.key, label: field.label, readable, hint: maskSecret(decrypted) }
}

/**
 * Every connection that holds a secret, with what it is worth to rotate.
 *
 * Connections with no stored secret are left out entirely rather than listed
 * empty — this page answers "what am I holding", and a connector that needs
 * no key is not an answer to it.
 */
export function listKeys(
  connections: SourceConnection[],
  authByConnector: Map<string, ConnectorConfigField[] | undefined>,
  workflows: WorkflowDefinition[],
  decryptedFor: (connectionId: string) => Record<string, string> | undefined
): ConnectorKey[] {
  const counts = usageCounts(workflows)
  const keys: ConnectorKey[] = []
  for (const connection of connections) {
    const decrypted = decryptedFor(connection.id) ?? {}
    const fields = passwordFields(authByConnector.get(connection.connectorId))
      .map((field) => describeField(field, connection.filters[field.key], decrypted[field.key]))
      .filter((field): field is ConnectorKeyField => field !== undefined)
    if (fields.length === 0) continue
    keys.push({
      connectionId: connection.id,
      name: connection.name,
      connectorId: connectionConnectorId(connection),
      fields,
      usageCount: counts.get(connection.id) ?? 0
    })
  }
  return keys.sort((a, b) => a.name.localeCompare(b.name))
}
