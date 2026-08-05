/**
 * Author-facing types for Vorn connectors.
 *
 * A connector written with this SDK runs as an ordinary MCP stdio server, so
 * it is shared as a normal npm package and installed by pointing a Vorn
 * connection at `npx -y <package>`. Nothing about the host app has to change
 * to accept a new connector.
 */

/** A raw item as the author's code returns it. Only id and title are required. */
export interface ConnectorItem {
  /** Stable upstream identity. Vorn dedupes on this, so it must not change. */
  externalId: string | number
  title: string
  url?: string
  description?: string
  /** Raw upstream status (`open`, `Active`, `In Progress`, …). */
  status?: string
  labels?: string[]
  assignee?: string
  /**
   * When the item last changed. Vorn advances its poll cursor from this field,
   * so it must be monotonic per item and comparable as an ISO 8601 string.
   * Defaults to poll time when omitted.
   */
  updatedAt?: string | Date
  /** Extra fields to expose to workflow templates as `{{trigger.item.<key>}}`. */
  data?: Record<string, unknown>
}

/** A connector item after normalization. This is the exact JSON Vorn sees. */
export interface NormalizedItem extends Record<string, unknown> {
  externalId: string
  title: string
  url: string
  description: string
  status: string
  labels: string[]
  updatedAt: string
  assignee?: string
}

/** Declares a value the connector needs at runtime, read from the environment. */
export interface ConnectorConfigField {
  key: string
  label: string
  /** Environment variable the value is read from. Defaults to CONSTANT_CASE(key). */
  env?: string
  required?: boolean
  /** Secrets are stored encrypted by Vorn and never printed by the CLI. */
  secret?: boolean
  description?: string
  default?: string
}

export type ConnectorConfig = Record<string, string | undefined>

export interface PollContext {
  config: ConnectorConfig
  /**
   * Lower bound the host asked for, when it was able to supply one. Treat it
   * as a hint: returning older items is safe because Vorn dedupes, but
   * returning fewer than everything after `since` loses events.
   */
  since?: string
  /** Opaque cursor previously returned by this trigger, when supplied. */
  cursor?: string
  /** Upper bound on items to return in one page. */
  limit?: number
  /** Injectable clock so tests are deterministic. */
  now(): string
}

export interface PollOutcome {
  items: ConnectorItem[]
  nextCursor?: string
  hasMore?: boolean
}

export interface TriggerDefinition {
  /** Event key, e.g. `workItemCreated`. Becomes the `poll_<type>` MCP tool. */
  type: string
  label: string
  description?: string
  poll(context: PollContext): Promise<PollOutcome> | PollOutcome
}

export interface ActionInputField {
  key: string
  label: string
  type?: 'string' | 'number' | 'boolean'
  required?: boolean
  description?: string
}

/**
 * A field the action is known to return. Declaring these is optional — extra
 * keys always pass through — but declared fields show up in Vorn's variable
 * autocomplete as `{{steps.<action>.<key>}}`.
 */
export interface ActionOutputField {
  key: string
  type?: 'string' | 'number' | 'boolean'
  description?: string
}

export interface ActionContext {
  config: ConnectorConfig
  now(): string
}

export interface ActionDefinition {
  /** Action key, e.g. `closeWorkItem`. Becomes an MCP tool of the same name. */
  type: string
  label: string
  description?: string
  inputs?: ActionInputField[]
  outputs?: ActionOutputField[]
  run(
    args: Record<string, unknown>,
    context: ActionContext
  ): Promise<Record<string, unknown> | void> | Record<string, unknown> | void
}

export interface ConnectorDefinition {
  /** Stable connector id, e.g. `azure-devops`. */
  id: string
  name: string
  version?: string
  description?: string
  config?: ConnectorConfigField[]
  triggers?: TriggerDefinition[]
  actions?: ActionDefinition[]
}

/** A validated definition. Every accessor below is guaranteed non-null. */
export interface Connector extends ConnectorDefinition {
  readonly version: string
  readonly config: ConnectorConfigField[]
  readonly triggers: TriggerDefinition[]
  readonly actions: ActionDefinition[]
}
