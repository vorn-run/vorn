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
  /**
   * A note for whoever is building this connector rather than using it: where
   * the value is found, what a good one looks like. The factory's agent reads
   * these, so a field that is easy to get wrong can say so once here instead of
   * being got wrong in every connector that copies it.
   */
  builderHint?: string
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
  /** Fetch with the SDK's retry and backoff applied. A poll is always a read. */
  fetch: typeof fetch
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
  /** Fetch with the SDK's retry and backoff applied. A fetch is always a read. */
  fetch: typeof fetch
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

/**
 * What kind of value an action argument takes.
 *
 * Every argument still arrives as a string — Vorn renders them from templates —
 * so this says how to read one, and how to draw its field. `select` is a
 * string with known choices; `json` is a string holding a structured value.
 */
export type ActionInputType = 'string' | 'number' | 'boolean' | 'select' | 'json'

/** One choice a `select` argument offers. */
export interface ActionInputOption {
  value: string
  /** Shown instead of the value where the raw value would not read well. */
  label?: string
}

export interface ActionInputField {
  key: string
  label: string
  type?: ActionInputType
  required?: boolean
  description?: string
  /** Fixed choices, for a `select` whose options are known when it is written. */
  options?: ActionInputOption[]
  /**
   * Names an options set the connector serves, for a `select` whose choices
   * are only knowable against a live connection — the channels in a workspace,
   * the projects in an account.
   */
  loadOptions?: string
  /** A note for whoever is building the connector, not for whoever runs it. */
  builderHint?: string
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
  /**
   * Fetch, with the SDK's retry, backoff and rate-limit handling already
   * applied. Prefer it over the global one: a hand-written action gets the
   * same resilience a declared request does, and tests can replace it.
   */
  fetch: typeof fetch
}

/**
 * One step of reshaping a response.
 *
 * Each op reads the whole value, or just what lives at its dotted `path`, and
 * leaves the rest alone. They compose left to right, which is enough to turn
 * most envelopes into the record a workflow step wants.
 */
export type PostReceiveOp =
  /** Keep only these keys, of the object or of every object in the list. */
  | { op: 'pick'; keys: string[]; path?: string }
  /** Give a key a better name, of the object or of every object in the list. */
  | { op: 'rename'; from: string; to: string; path?: string }
  /** Replace the whole value with what is at this path — unwrap the envelope. */
  | { op: 'flatten'; path: string }
  /** Keep the list entries whose `key` equals this value. */
  | { op: 'filter'; key: string; equals: unknown; path?: string }
  /** Run these ops against every entry of the list. */
  | { op: 'map'; ops: PostReceiveOp[]; path?: string }

/**
 * How to ask for the page after this one.
 *
 * Declared rather than written because every source does the same three things
 * — hand back a cursor, count pages, or put a link in a header — and following
 * them by hand is where "only the first 100 items ever arrive" comes from.
 */
export type PaginationStrategy =
  /** The response carries a cursor at `cursorPath`; send it back as `param`. */
  | { kind: 'cursor'; cursorPath: string; param: string; itemsPath?: string }
  /** Ask for page 1, 2, 3 … under `param`, until a page comes back short. */
  | { kind: 'page'; param: string; startPage?: number; itemsPath?: string }
  /** Follow the `Link` header's `rel="next"`, as paged HTTP APIs do. */
  | { kind: 'link'; itemsPath?: string }

/** An HTTP call an action makes, with `{{args.x}}` and `{{config.y}}` filled in. */
export interface ActionRequest {
  /** Defaults to GET. */
  method?: 'GET' | 'HEAD' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  url: string
  headers?: Record<string, string>
  /** Query parameters. An argument that resolves to nothing is left out. */
  query?: Record<string, string>
  /** Sent as JSON unless it is already a string, or a content type says otherwise. */
  body?: unknown
  /** Follow every page rather than returning only the first. */
  paginate?: PaginationStrategy
}

interface ActionBase {
  /** Action key, e.g. `closeWorkItem`. Becomes an MCP tool of the same name. */
  type: string
  label: string
  description?: string
  /**
   * Whether repeating the call with the same arguments is safe. Surfaced in
   * the MCP tool description, because an agent retrying a failed step has no
   * other way to know whether it is about to create a second issue — and it is
   * what decides whether the SDK may retry the call itself.
   */
  idempotent?: boolean
  inputs?: ActionInputField[]
  outputs?: ActionOutputField[]
}

/**
 * An action is either declared or hand-written, never both — the same union
 * shape triggers use, so the invalid combination is a type error while the
 * connector is being written rather than a throw once it is installed.
 */
export type ActionDefinition = ActionBase &
  (
    | {
        run(
          args: Record<string, unknown>,
          context: ActionContext
        ): Promise<Record<string, unknown> | void> | Record<string, unknown> | void
        request?: never
        postReceive?: never
      }
    | {
        /** The call to make. The SDK sends it and keeps the response. */
        request: ActionRequest
        /** How to reshape what came back, before the step sees it. */
        postReceive?: PostReceiveOp[]
        run?: never
      }
  )

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

/**
 * What a connector reports about its own readiness.
 *
 * `message` is shown to the user verbatim, so it should say what to do rather
 * than what went wrong — "run `gh auth login`" beats "not authenticated".
 */
export interface PreflightResult {
  ok: boolean
  message?: string
}

/**
 * How a connector signs in, lowest rung first.
 *
 * `none` needs nothing — installing it is the whole setup. `cli` borrows a
 * login that already works on the machine, which is the rung to prefer
 * whenever a mature tool is signed in anyway. `key` asks for a credential.
 * `oauth` is declared but not yet carried by the host.
 */
export type AuthRung = 'none' | 'cli' | 'key' | 'oauth'

/**
 * What a connector needs before it can talk to anything.
 *
 * Declaring this is what lets the app say how a connector signs in *before*
 * anyone installs it, and lets a `cli` connector show who you already are
 * instead of a token field. The credential itself is never described here —
 * only where it comes from.
 */
export interface ConnectorAuth {
  rung: AuthRung
  /**
   * Asks the borrowed tool whether it is signed in, e.g. `glab auth status`.
   * Required for `cli`: without it the app has nothing to ask.
   */
  probe?: { command: string; args?: string[] }
  /**
   * What to take from the signed-in tool. `env` names variables to pass
   * through; `tokenArgs` is a command that prints a token, run fresh at spawn
   * so nothing is ever stored.
   */
  borrow?: { env?: string[]; tokenArgs?: string[] }
  /** Config field keys holding the credential. Required for `key`. */
  keys?: string[]
}

/** What an options set is given to work out its choices. */
export interface OptionsContext {
  config: ConnectorConfig
  now(): string
  /** Fetch with the SDK's retry and backoff applied. Listing choices is a read. */
  fetch: typeof fetch
}

/**
 * Answers the question "what can this field be?" against a live connection.
 *
 * A bare string is taken as a choice that shows itself; return the object form
 * when the value a step should send and the words a person should read differ.
 */
export type OptionsLoader = (
  context: OptionsContext
) => Promise<Array<ActionInputOption | string>> | Array<ActionInputOption | string>

export interface ConnectorDefinition {
  /** Stable connector id, e.g. `azure-devops`. */
  id: string
  name: string
  version?: string
  description?: string
  icon?: ConnectorIcon
  config?: ConnectorConfigField[]
  /**
   * Named sets of choices an input can point at with `loadOptions`, for fields
   * whose values only exist against a live connection — the channels in a
   * workspace, the projects in an account.
   */
  options?: Record<string, OptionsLoader>
  /**
   * How this connector signs in. Absent means the app cannot say, which reads
   * as "a key, probably" — declare it rather than leave that to be guessed.
   */
  auth?: ConnectorAuth
  triggers?: TriggerDefinition[]
  actions?: ActionDefinition[]
  /**
   * Whether this connector could work right now, asked before anyone waits on
   * a poll.
   *
   * A connector whose credentials come from config fields does not need this:
   * a missing field is already a visible, nameable error. One that borrows an
   * external tool's login — `gh auth login`, `az login` — has no field to be
   * missing, so without this the first sign that the tool is absent or signed
   * out is a poll failing some minutes after the connection was saved.
   *
   * Answer `ok: false` with a message saying what to do about it. Throwing is
   * equivalent — the server catches it and reports the same shape with the
   * error's message — so there is one result for a caller to read and no
   * behaviour riding on which you choose. Prefer returning when the state is
   * one you recognise, because then you get to write the sentence.
   *
   * Absent means there is nothing to check, which is not the same answer as a
   * check that passed.
   */
  preflight?(): Promise<PreflightResult> | PreflightResult
}

/** A validated definition. Every accessor below is guaranteed non-null. */
export interface Connector extends ConnectorDefinition {
  readonly version: string
  readonly config: ConnectorConfigField[]
  readonly triggers: TriggerDefinition[]
  readonly actions: ActionDefinition[]
}
