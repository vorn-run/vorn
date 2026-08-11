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

/**
 * How the SDK decides which fetched items are new.
 *
 * - `timestamp` — for sources that expose a reliable "last changed" field and
 *   can filter on it. Handles the boundary case where several items share the
 *   newest timestamp, which is the classic source of both duplicates and
 *   silently dropped items.
 * - `lastItem` — for feeds that return newest-first with no dependable
 *   timestamp. The cursor is the newest id already delivered.
 */
export type DedupeStrategy = 'timestamp' | 'lastItem'

/**
 * What a declarative trigger's `fetch` receives. Deliberately smaller than
 * {@link PollContext}: cursor encoding, ordering, windowing and de-duplication
 * are the SDK's job, so the author only has to answer "what is there now?".
 */
export interface FetchContext {
  config: ConnectorConfig
  /**
   * With `dedupe: 'timestamp'`, everything changed at or after this instant is
   * worth returning. Absent on the very first poll. Returning a little too
   * much is safe — the SDK drops what was already delivered.
   */
  since?: string
  /**
   * With `dedupe: 'lastItem'`, the newest id already delivered. Absent on the
   * very first poll. Return the feed newest-first and the SDK will stop there.
   */
  lastItemId?: string
  /** Upper bound on items worth returning in one call. */
  limit?: number
  /** Injectable clock so tests are deterministic. */
  now(): string
}

/**
 * What an upstream state should become when an item is imported as a task.
 *
 * A suggestion, not a rule: it seeds the connection form, and the person
 * setting it up can change it. Without any, everything a connector imports
 * lands as `todo` regardless of whether it was closed a year ago.
 */
export interface StatusSuggestion {
  /** The value the connector reports in `ConnectorItem.status`. */
  upstream: string
  suggestedLocal: 'todo' | 'in_progress' | 'in_review' | 'done' | 'cancelled'
}

/**
 * The workflow to create when a connection is made.
 *
 * A connector that fires on a schedule is useless until something polls it, and
 * expecting every person to build that workflow by hand is how a connection
 * ends up configured and silent. Seeded workflows are ordinary, visible and
 * editable — the schedule is a starting point, not a fixed rule.
 */
export interface DefaultWorkflow {
  name: string
  defaultCronFromMinutes: number
}

interface TriggerBase {
  /** Event key, e.g. `workItemCreated`. Becomes the `poll_<type>` MCP tool. */
  type: string
  label: string
  description?: string
  /** Seeds the connection's status mapping; the person setting it up owns it. */
  statusMapping?: StatusSuggestion[]
  /** Seeds a polling workflow when a connection is created. */
  defaultWorkflow?: DefaultWorkflow
  /**
   * Representative items. `vorn-connector check` replays these through the
   * real dedupe pipeline, so a connector can be verified before anyone has
   * credentials for it.
   */
  sample?: ConnectorItem[]
}

/**
 * A trigger is either declarative or hand-written, never both — expressed as a
 * union so the invalid combinations are a type error at authoring time rather
 * than a throw when the connector is first loaded.
 */
export type TriggerDefinition = TriggerBase &
  (
    | {
        /**
         * Declarative polling: return what the source has and let the SDK
         * handle cursors and de-duplication.
         */
        dedupe: DedupeStrategy
        fetch(context: FetchContext): Promise<ConnectorItem[]> | ConnectorItem[]
        poll?: never
      }
    | {
        /**
         * Full control over cursors and paging. Use only when the source's
         * paging cannot be expressed as "give me everything since X".
         */
        poll(context: PollContext): Promise<PollOutcome> | PollOutcome
        dedupe?: never
        fetch?: never
      }
  )

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
  /**
   * Whether repeating the call with the same arguments is safe. Surfaced in
   * the MCP tool description, because an agent retrying a failed step has no
   * other way to know whether it is about to create a second issue.
   */
  idempotent?: boolean
  inputs?: ActionInputField[]
  outputs?: ActionOutputField[]
  run(
    args: Record<string, unknown>,
    context: ActionContext
  ): Promise<Record<string, unknown> | void> | Record<string, unknown> | void
}

/**
 * A connector's own glyph, so an installed connector is recognizable in a list
 * rather than sharing one generic icon with every other one.
 *
 * Path data only — deliberately not markup. Vorn draws these itself as
 * `<path d="...">` inside an `<svg>` it owns, so a connector cannot inject
 * elements, scripts or external references into the app rendering it.
 */
export interface ConnectorIcon {
  /** Defaults to `0 0 24 24`. */
  viewBox?: string
  /** SVG path `d` data, drawn with `fill="currentColor"` so it inherits color. */
  paths: string[]
}

export interface ConnectorDefinition {
  /** Stable connector id, e.g. `azure-devops`. */
  id: string
  name: string
  version?: string
  description?: string
  icon?: ConnectorIcon
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
