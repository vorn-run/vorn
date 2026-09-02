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
  WorkflowDefinition
} from '@vornrun/shared/types'
import { connectionConnectorId } from '@vornrun/shared/types'
import { boundConnectionKey } from '@vornrun/shared/workflow-portability'

/** The one field that carries a set of env vars rather than a single value. */
export const SECRET_ENV_FIELD = 'secretEnv'

/** Which of a connector's fields hold secrets, as the connector itself says. */
export function passwordFields(auth: ConnectorConfigField[] | undefined): ConnectorConfigField[] {
  return (auth ?? []).filter((field) => field.type === 'password')
}

/**
 * Enough of a value to recognize it by, never enough to use it.
 *
 * The opening is how a person tells a live key from a test one; the tail is
 * the part that would let someone reconstruct it, so only the head travels.
 */
export function maskSecret(value: string | undefined): string {
  if (!value) return ''
  if (value.length < 12) return '••••'
  return `${value.slice(0, 7)}…`
}

/** The env names a packaged connector's blob carries, when it can be read. */
export function envNamesOf(blob: string | undefined): string[] {
  if (!blob) return []
  try {
    const parsed: unknown = JSON.parse(blob)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return []
    return Object.keys(parsed as Record<string, unknown>).filter((name) => name !== '')
  } catch {
    return []
  }
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
      const key = boundConnectionKey(node, config)
      if (key === null) continue
      const bound = config[key]
      if (typeof bound !== 'string' || bound === '') continue
      counts.set(bound, (counts.get(bound) ?? 0) + 1)
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
