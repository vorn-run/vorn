// Type-only, so the portability module can keep importing values from here.
import type { PortableWorkflow } from './workflow-portability'

/** AI agents only. Use this for icon maps, install status, command configs, and
 *  anything else that applies exclusively to an AI CLI — not to plain shells. */
export type AiAgentType = 'claude' | 'copilot' | 'codex' | 'opencode' | 'gemini'

/** Any session type that can back a terminal tab. `'shell'` is a plain PTY
 *  (zsh/bash), everything else is an AI agent. */
export type AgentType = AiAgentType | 'shell'

export type AgentStatus = 'running' | 'waiting' | 'idle' | 'error'

/** Narrowing type guard: true when the session is an AI agent (not a plain shell). */
export function isAiAgent(agentType: AgentType | undefined): agentType is AiAgentType {
  return agentType !== undefined && agentType !== 'shell'
}

/**
 * True for sessions Vorn renders itself — a plain pty whose output is ordinary
 * scrollback, so command boundaries, the spine and the input bar all apply.
 * Agent sessions paint their own full-screen interface and are excluded.
 */
export function isShellSession(agentType: AgentType | undefined): boolean {
  return agentType === 'shell'
}

export function supportsExactSessionResume(agentType: AgentType): boolean {
  return agentType !== 'gemini' && agentType !== 'shell'
}

/** Can we pin a pre-generated session ID on fresh launch so we can --resume it later? */
export function supportsSessionIdPinning(agentType: AgentType): boolean {
  return agentType === 'claude' || agentType === 'copilot'
}

/** CLI flag used to pin a pre-generated session ID on fresh launch. Only valid
 *  when supportsSessionIdPinning(agentType) is true. */
export function getSessionIdPinningFlag(agentType: AgentType): string {
  switch (agentType) {
    case 'claude':
      return '--session-id'
    case 'copilot':
      return '--session-id'
    default:
      throw new Error(`getSessionIdPinningFlag: ${agentType} does not support session ID pinning`)
  }
}

export function getRecentSessionActivityLabel(agentType: AgentType): string {
  switch (agentType) {
    case 'claude':
      return 'entry'
    case 'codex':
      return 'entry'
    case 'copilot':
      return 'turn'
    case 'gemini':
      return 'prompt'
    case 'opencode':
      return 'message'
    case 'shell':
      return 'line'
  }
}

export interface AgentCommandConfig {
  command: string
  args: string[]
  headlessArgs?: string[]
  fallbackCommand?: string
  fallbackArgs?: string[]
}

export interface TerminalSession {
  id: string
  agentType: AgentType
  projectName: string
  projectPath: string
  status: AgentStatus
  createdAt: number
  pid: number
  displayName?: string
  branch?: string
  worktreePath?: string
  worktreeName?: string
  isWorktree?: boolean
  remoteHostId?: string
  remoteHostLabel?: string
  hookSessionId?: string
  agentSessionId?: string
  statusSource?: 'hooks' | 'pattern'
  /**
   * The geometry the PTY is currently running at.
   *
   * Held because a program renders against it: wrap points, cursor position and
   * every full-screen repaint are decided by these numbers, so anything that
   * models the screen has to agree with them exactly. Nothing recorded them
   * before -- all three spawn sites passed 80x24 to node-pty and `resizePty`
   * forwarded new values without keeping them.
   *
   * Last writer wins between attached clients, because node-pty already works
   * that way: a phone fitting to 60x20 and a desktop at 200x50 will fight, and
   * whichever resized last is what the program is drawing against. That is a
   * pre-existing behaviour, and this records it rather than changing it.
   *
   * Optional because not every `TerminalSession` has a PTY behind it -- a
   * workflow builds a synthetic one to describe its source, and inventing a
   * geometry for something that is not being drawn would be a fact nobody
   * checked. Absent means "whatever a PTY starts at".
   */
  cols?: number
  rows?: number
  /** Shell session only: working directory the PTY was started in. */
  shellCwd?: string
  /** HEAD where it was working, refreshed as it works, so a restore can tell the tree moved. */
  headCommit?: string
  /** Shell session only: PTY exit code once the shell has exited. */
  shellExitCode?: number
  /**
   * When the record was last written down.
   *
   * Set by the database on save and read back by `getPreviousSessions`; a
   * session that has never been persisted does not have one. It is the only
   * thing on disk that says roughly when a run ended, which is what a pane
   * restored from a previous process has to tell somebody.
   */
  savedAt?: number
}

/**
 * A session from a previous run that no pane has taken yet.
 *
 * Held by the server rather than the client so two of them can be looking at the
 * same one: the first to claim it gets it, and the second is told it is gone
 * rather than starting a second agent against one transcript.
 */
export interface RestoredSession {
  session: TerminalSession
  /** Roughly when it ended: the last save the previous process managed. */
  endedAt: number
  /** Whether a screen was rebuilt for it, so a pane has something to show. */
  replayable: boolean
  /** The recorded history stops short of what actually happened. */
  partial: boolean
  /**
   * The last run shut down rather than being stopped under it.
   *
   * "You closed Vorn" and "something stopped it" are different events, and a
   * pane that reports the second for the first reads like a fault report for an
   * ordinary quit.
   */
  closedCleanly: boolean
  /** The machine went down under it: saved before this boot, and never closed. */
  rebooted: boolean
  /** What is there now against what was recorded. Absent for remote sessions. */
  environment?: RestoreEnvironment
}

export interface RestoreEnvironment {
  worktree: 'ok' | 'missing'
  branch: { recorded: string | null; actual: string | null }
  head: { recorded: string | null; actual: string | null }
}

/** Both commits known and different. Unknown on either side is not a move. */
export function headMoved(env: RestoreEnvironment | undefined): boolean {
  if (!env) return false
  const { recorded, actual } = env.head
  return recorded !== null && actual !== null && recorded !== actual
}

export type AuthMethod = 'key-file' | 'key-stored' | 'password' | 'agent'

export interface SSHKey {
  id: string
  label: string
  /** Base64-encoded safeStorage-encrypted private key */
  encryptedPrivateKey: string
  publicKey?: string
  certificate?: string
  keyType?: string
  createdAt: string
}

export interface SSHKeyMeta {
  id: string
  label: string
  keyType?: string
  publicKey?: string
  createdAt: string
}

export interface RemoteHost {
  id: string
  label: string
  hostname: string
  user: string
  port: number
  authMethod?: AuthMethod
  sshKeyPath?: string
  credentialId?: string
  encryptedPassword?: string
  sshOptions?: string
}

/**
 * Who is operating Vorn. A single owner is seeded on first run — this is an
 * identity, not a login, and it exists so that the local desktop can
 * authenticate as somebody rather than being exempt from authentication.
 */
export type UserRole = 'owner'

export interface User {
  id: string
  name: string
  role: UserRole
  createdAt: string
}

/**
 * A credential belonging to one device. Deliberately carries no secret: the
 * plaintext is returned once at creation and only its hash is ever stored, so
 * there is nothing here for a listing to leak.
 */
export interface DeviceToken {
  id: string
  userId: string
  /** Human label, e.g. "Javier's iPhone". */
  name: string
  createdAt: string
  lastSeenAt: string | null
  revokedAt: string | null
}

/**
 * A phone that offered a valid pairing code and is waiting to be approved.
 *
 * Carries what the person needs in order to recognise it. Approving is the
 * step that stops a code someone photographed off a screen from becoming a
 * token, so the prompt has to say which device is asking and from where.
 */
export interface PairingRequest {
  requestId: string
  /** What the device calls itself. */
  deviceName: string
  /** The address it reached the server from. */
  address: string
  askedAt: number
  status: 'pending' | 'approved' | 'denied' | 'collected'
}

export interface WorkspaceConfig {
  id: string // 'personal' for default, UUID for user-created
  name: string
  icon?: string
  iconColor?: string
  order: number
}

export const DEFAULT_WORKSPACE: WorkspaceConfig = {
  id: 'personal',
  name: 'Personal',
  icon: 'User',
  iconColor: '#6b7280',
  order: 0
}

export interface ProjectConfig {
  name: string
  path: string
  /** AI agents preferred for this project. Shells are launched directly, not via preferences. */
  preferredAgents: AiAgentType[]
  icon?: string
  iconColor?: string
  hostIds?: string[] // 'local' | remote host UUIDs; absent = ['local']
  workspaceId?: string // defaults to 'personal' if absent
}

export function getProjectHostIds(project: ProjectConfig): string[] {
  return project.hostIds?.length ? project.hostIds : ['local']
}

/** Returns the first remote host ID for a project, or undefined if local-only. */
export function getProjectRemoteHostId(project: ProjectConfig): string | undefined {
  return getProjectHostIds(project).find((id) => id !== 'local')
}

// Task queue types
export type TaskStatus = 'todo' | 'in_progress' | 'in_review' | 'done' | 'cancelled'

export function isTerminalTaskStatus(status: TaskStatus): boolean {
  return status === 'done' || status === 'cancelled'
}

export type TaskViewMode = 'list' | 'kanban'

export const MINIMIZED_PLACEMENTS = ['canvas', 'toolbar', 'both'] as const
export type MinimizedPlacement = (typeof MINIMIZED_PLACEMENTS)[number]

export type MainViewMode = 'sessions' | 'tasks' | 'workflows'

export interface TaskConfig {
  id: string
  projectName: string
  title: string
  description: string
  status: TaskStatus
  order: number
  assignedSessionId?: string
  assignedAgent?: AiAgentType
  agentSessionId?: string // Real agent session ID (e.g. Claude session_id from hooks) for resume
  branch?: string
  useWorktree?: boolean
  worktreePath?: string
  images?: string[] // filenames relative to task-images/{taskId}/
  createdAt: string
  updatedAt: string
  completedAt?: string
  archivedAt?: string
  // Source connector fields (set when task originates from an external connector)
  sourceConnectorId?: string // 'github' | 'linear' | custom connector id
  sourceExternalId?: string // e.g. issue number "42"
  sourceExternalUrl?: string // link to upstream item
}

// ─── Connector System ───────────────────────────────────────────

/** Origin of a task mutation — only 'user' fires workflow triggers. */
export type MutationOrigin = 'user' | 'sync' | 'system'

/** Stable id for a connector-seeded workflow tied to a (connection × event). */
export function connectorSeededWorkflowId(connectionId: string, event: string): string {
  return `connector:${connectionId}:${event}`
}

/** Prefix used to find all seeded workflows for a given connection. */
export function connectorSeededWorkflowIdPrefix(connectionId: string): string {
  return `connector:${connectionId}:`
}

/** Inverse of connectorSeededWorkflowId — parses the id back into its parts,
 *  or returns null if the id isn't a seeded-connector id. */
export function parseConnectorWorkflowId(
  id: string
): { connectionId: string; event: string } | null {
  if (!id.startsWith('connector:')) return null
  const rest = id.slice('connector:'.length)
  const colon = rest.indexOf(':')
  if (colon === -1) return null
  return { connectionId: rest.slice(0, colon), event: rest.slice(colon + 1) }
}

// -- Connector interface --

export interface ExternalItem {
  externalId: string
  url: string
  title: string
  description: string
  status: string // raw upstream status
  labels?: string[]
  assignee?: string
  priority?: string
  updatedAt: string
  metadata?: Record<string, unknown>
}

export interface ExternalItemPage {
  items: ExternalItem[]
  nextCursor?: string
  hasMore?: boolean
}

export interface PollResult {
  events: TriggerEvent[]
  nextCursor?: string
  /** More remote pages are immediately available. The scheduler keeps
   * pulling bounded pages while the connector advances `nextCursor`. */
  hasMore?: boolean
}

export interface TriggerEvent {
  id: string // dedup key
  type: string
  data: Record<string, unknown>
  timestamp: string
}

export interface ActionResult {
  success: boolean
  output?: Record<string, unknown>
  error?: string
}

export interface ConnectorConfigField {
  key: string
  label: string
  type: 'text' | 'select' | 'multiselect' | 'toggle' | 'textarea' | 'password'
  required?: boolean
  placeholder?: string
  description?: string
  options?: { value: string; label: string }[]
  supportsTemplates?: boolean
}

export interface ConnectorTriggerDef {
  type: string // e.g. 'issueCreated'
  label: string
  description?: string
  configFields: ConnectorConfigField[]
  defaultIntervalMs: number
}

export interface ConnectorActionDef {
  type: string // e.g. 'createIssue'
  label: string
  description?: string
  configFields: ConnectorConfigField[]
  /**
   * JSON Schema describing the shape of `ActionResult.output` on success.
   * Used by the workflow editor to surface typed fields in the variable
   * autocomplete (so `{{steps.createIssue.html_url}}` shows up), and by
   * the template resolver to walk nested paths into the returned object.
   * Optional — actions without a declared schema fall back to the default
   * `output / status / error` keys every step has.
   */
  outputSchema?: Record<string, unknown>
}

export interface ConnectorStatusOption {
  upstream: string
  suggestedLocal: TaskStatus
}

export interface ConnectorManifest {
  auth: ConnectorConfigField[]
  /**
   * Name the connection after a git repository, detected from the project.
   *
   * The add-connection form then offers the owner/repo of whichever project is
   * selected, falling back to typing them in. Declared rather than inferred from
   * the connector's id, which is what it used to be -- so a connector that
   * arrives as a pack can ask for it, and the app has no list of which services
   * happen to live in git.
   */
  detectRepo?: boolean
  taskFilters?: ConnectorConfigField[]
  statusMapping?: ConnectorStatusOption[]
  triggers?: ConnectorTriggerDef[]
  actions?: ConnectorActionDef[]
  /**
   * Declarative default workflows seeded when a connection is created. Each
   * entry becomes a real WorkflowDefinition with a `connectorPoll` trigger and
   * a `createTaskFromItem` node — fully visible and editable in the workflow
   * editor. The seeded workflow's id is stable
   * (`connector:{connectionId}:{event}`) so delete sticks.
   */
  defaultWorkflows?: Array<{
    name: string
    /** Event key matching one of `triggers[].type`. */
    event: string
    /** Default cron derived from this minute interval when the workflow is seeded. */
    defaultCronFromMinutes: number
    /** The downstream node the seeded workflow wires to. Only one supported today. */
    downstream: 'createTaskFromItem'
  }>
}

/**
 * The core connector interface. A connector provides tasks, triggers, and/or
 * actions for an external service. Implementations can use any transport:
 * gh CLI, REST API, MCP server, shell script, etc.
 */
export interface VornConnector {
  readonly id: string
  readonly name: string
  readonly icon: string
  readonly capabilities: ('tasks' | 'triggers' | 'actions')[]

  listItems?(filters: Record<string, unknown>): Promise<ExternalItem[]>
  /** Bounded reconciliation page. Connectors that implement this let manual
   * backfill drain the complete remote result set without one huge response. */
  listItemsPage?(filters: Record<string, unknown>, cursor?: string): Promise<ExternalItemPage>
  getItem?(externalId: string, filters: Record<string, unknown>): Promise<ExternalItem | null>
  poll?(triggerType: string, config: Record<string, unknown>, cursor?: string): Promise<PollResult>
  execute?(actionType: string, args: Record<string, unknown>): Promise<ActionResult>

  describe(): ConnectorManifest
}

// -- Source connection (saved config for a linked connector) --

export interface SourceConnection {
  id: string
  connectorId: string
  name: string
  filters: Record<string, unknown>
  syncIntervalMinutes: number
  statusMapping: Record<string, TaskStatus>
  executionProject?: string // vorn project for tasks
  lastSyncAt?: string
  lastSyncError?: string
  syncCursor?: string
  createdAt: string
}

/** One stored secret on a connection, described without being disclosed. */
export interface ConnectorKeyField {
  key: string
  label: string
  /** False when the ciphertext is there but this machine cannot read it. */
  readable: boolean
  /** Enough of the value to recognize it by, for a single-value field. */
  hint?: string
  /** The env names carried, for a field that holds a set of them. */
  envNames?: string[]
}

/** A connection seen as what it holds, rather than as what it connects to. */
export interface ConnectorKey {
  connectionId: string
  name: string
  /** The real connector id, unwrapped from the `mcp` a package is stored as. */
  connectorId: string
  fields: ConnectorKeyField[]
  /** Workflow steps that run against this connection. */
  usageCount: number
}

/**
 * Where a packaged connector records itself on the connection that runs it.
 *
 * A connector installed from a package is stored as an `mcp` connection, so
 * `connectorId` is `mcp` for every one of them and its real identity, version
 * and icon have to travel in `filters` instead. Both the desktop app and the
 * MCP server create these connections, so the key names live here rather than
 * being spelled out at each site — a disagreement between a writer and a
 * reader is invisible until a connection shows the wrong icon or is counted
 * against the wrong connector.
 */
export const SDK_FILTER_KEYS = {
  connectorId: 'sdkConnectorId',
  version: 'sdkVersion',
  icon: 'sdkIcon',
  implicit: 'implicit'
} as const

/** Whether the app made this connection itself for a connector that asks for nothing. */
export function isImplicitConnection(connection: {
  filters: SourceConnection['filters']
}): boolean {
  return connection.filters?.[SDK_FILTER_KEYS.implicit] === true
}

// Credential-shaped names no third-party process is handed, and the agent markers stripped for everyone.
export const SENSITIVE_ENV_PREFIXES = [
  'AWS_SECRET',
  'AWS_SESSION',
  'GITHUB_TOKEN',
  'GH_TOKEN',
  'OPENAI_API',
  'ANTHROPIC_API',
  'GOOGLE_API',
  'STRIPE_',
  'DATABASE_URL',
  'DB_PASSWORD',
  'SECRET_',
  'PRIVATE_KEY',
  'NPM_TOKEN',
  'NODE_AUTH_TOKEN'
]
export const NEVER_BORROWED_ENV = { keys: ['CLAUDECODE'], prefixes: ['CLAUDE_CODE_'] }

export function isNeverBorrowedEnvName(name: string): boolean {
  const upper = name.toUpperCase()
  return (
    NEVER_BORROWED_ENV.keys.includes(upper) ||
    NEVER_BORROWED_ENV.prefixes.some((p) => upper.startsWith(p))
  )
}

export function isCredentialEnvName(name: string): boolean {
  const upper = name.toUpperCase()
  return SENSITIVE_ENV_PREFIXES.some((p) => upper.startsWith(p))
}

/** What a pack will actually be handed: declared, never stripped, never a credential by name. */
export function borrowableFromManifest(
  auth: SdkConnectorAuth | undefined,
  env: ReadonlyArray<{ name: string }>
): string[] {
  return declaredBorrows(auth, env).filter(
    (name) => !isNeverBorrowedEnvName(name) && !isCredentialEnvName(name)
  )
}

/** The borrow names a manifest can honour, in the casing it declared them: what it reads is what it gets. */
export function declaredBorrows(
  auth: SdkConnectorAuth | undefined,
  env: ReadonlyArray<{ name: string }>
): string[] {
  const declared = new Map(env.map((entry) => [entry.name.toUpperCase(), entry.name]))
  return (auth?.borrow?.env ?? []).flatMap((name) => declared.get(name.toUpperCase()) ?? [])
}

/** What asking a borrowed tool answered; `ok: null` is "nothing to ask". */
export interface AuthProbeReport {
  ok: boolean | null
  identity?: string
  message?: string
  installHint?: string
}

/** The connector a connection belongs to, which for a package is not `mcp`. */
export function connectionConnectorId(connection: {
  connectorId: string
  filters: SourceConnection['filters']
}): string {
  const packaged = connection.filters?.[SDK_FILTER_KEYS.connectorId]
  return typeof packaged === 'string' && packaged !== '' ? packaged : connection.connectorId
}

// -- Task source link (sync metadata, separate from TaskConfig) --

export interface TaskSourceLink {
  taskId: string
  connectionId: string
  connectorId: string
  externalId: string
  externalUrl: string
  sourceStatusRaw: string
  sourceUpdatedAt: string
  lastSyncedAt: string
  conflictState: 'none' | 'upstream_changed' | 'both_changed'
}

// Session event types (lifecycle activity log)
export type SessionEventType = 'created' | 'exited' | 'renamed'

export interface SessionEvent {
  id?: number
  sessionId: string
  eventType: SessionEventType
  timestamp: string
  metadata?: Record<string, unknown>
}

// --- Workflow engine types (Logic Apps-style) ---

/** A single external item pulled by a connector poll and fanned out as its own
 *  workflow execution. Kept small and serializable so the engine's existing
 *  persist-and-resume pattern keeps working. */
export interface ConnectorItemContext {
  /** Durable delivery row. Internal to Vorn; connectors do not set it. */
  inboxId?: number
  /** Identifies the current delivery lease so stale windows cannot acknowledge it. */
  inboxLeaseToken?: string
  connectionId: string
  connectorId: string
  externalId: string
  externalUrl?: string
  title: string
  body?: string
  /** Full upstream payload for downstream template expansion. */
  raw: Record<string, unknown>
}

// Execution context passed from triggers to the execution engine
export interface WorkflowExecutionContext {
  task?: TaskConfig
  /**
   * Terminal session that launched a contextual workflow (right-click on a
   * card or terminal). Drives the `{{context.*}}` namespace alongside `task`.
   */
  source?: TerminalSession
  trigger?: {
    type: TriggerConfig['triggerType']
    fromStatus?: TaskStatus
    toStatus?: TaskStatus
    /** Webhook runs: the received request, for {{trigger.body.*}} / {{trigger.headers.*}}. */
    body?: unknown
    headers?: Record<string, string>
    query?: Record<string, string>
    method?: string
  }
  connectorItem?: ConnectorItemContext
  /**
   * Values the user supplied when starting the run, keyed by
   * `WorkflowInputDef.key`. Drives the `{{inputs.*}}` namespace.
   *
   * Values are `unknown` rather than `string` on purpose: a scalar input
   * resolves to a scalar, but a resource-backed input (a GitHub issue picked
   * from a connection) stores the whole item so templates can reach into it
   * with `{{inputs.issue.number}}`.
   */
  inputs?: Record<string, unknown>
}

export type WorkflowNodeType =
  | 'trigger'
  | 'launchAgent'
  | 'script'
  | 'condition'
  | 'approval'
  | 'createTaskFromItem'
  | 'callConnectorAction'
  | 'httpRequest'
  | 'loop'

export interface WorkflowNodePosition {
  x: number
  y: number
}

/**
 * Field types a manual-run input can take. The scalar types render as plain
 * form controls; `project` and `branch` reuse the pickers the run dialog
 * already has.
 *
 * A connection-backed picker (a GitHub issue / PR / repo) is intended next.
 * It is deliberately absent from this union until it has a producer: input
 * values are already `unknown`, so storing a whole resolved item needs no
 * type change here, and a member no editor can create would force every
 * future exhaustive switch to carry an untestable branch.
 */
export type WorkflowInputType =
  | 'text'
  | 'textarea'
  | 'number'
  | 'select'
  | 'boolean'
  | 'project'
  | 'branch'

/**
 * One parameter the user fills in when starting a manual run. Declared on the
 * trigger, so it travels with the workflow definition and needs no schema
 * change; supplied values land in `WorkflowExecutionContext.inputs` and expand
 * as `{{inputs.<key>}}` anywhere a template is accepted.
 */
export interface WorkflowInputDef {
  /** Template key — `{{inputs.<key>}}`. Must be a valid identifier. */
  key: string
  label: string
  type: WorkflowInputType
  required?: boolean
  /** Pre-filled value in the run dialog, as authored in the editor. */
  defaultValue?: string
  /** Choices for `type: 'select'`. Ignored otherwise. */
  options?: { value: string; label: string }[]
  placeholder?: string
  description?: string
}

// Trigger configs (discriminated union)
export interface ManualTriggerConfig {
  triggerType: 'manual'
  /**
   * Contextual workflows inherit folder/branch/worktree from the source that
   * launched them (a card or terminal right-click). They appear only in the
   * card and terminal context menus; from the sidebar/palette the user is
   * prompted for the source via SourcePromptDialog.
   */
  contextual?: boolean
  /**
   * Parameters the user is prompted for before the run starts. A workflow
   * that declares any of these always opens the run dialog, contextual or
   * not — there is no other moment at which the values could be supplied.
   */
  inputs?: WorkflowInputDef[]
}
export interface OnceTriggerConfig {
  triggerType: 'once'
  runAt: string
}
export interface RecurringTriggerConfig {
  triggerType: 'recurring'
  cron: string
  timezone?: string
}
export interface TaskCreatedTriggerConfig {
  triggerType: 'taskCreated'
  projectFilter?: string
}
export interface TaskStatusChangedTriggerConfig {
  triggerType: 'taskStatusChanged'
  projectFilter?: string
  fromStatus?: TaskStatus
  toStatus?: TaskStatus
}
/** Polls a connector on cron. Scheduler calls connector.poll(), updates the
 *  connection's cursor, and fires one workflow execution per new item. */
export interface ConnectorPollTriggerConfig {
  triggerType: 'connectorPoll'
  connectionId: string
  /** Event type from the connector manifest — e.g. 'issueCreated'. */
  event: string
  cron: string
  timezone?: string
}
/** Fires when the server's localhost webhook route receives a matching request. */
export interface WebhookTriggerConfig {
  triggerType: 'webhook'
  method: 'POST' | 'GET'
  /** Per-trigger secret path segment; requests without it are rejected. */
  token: string
}
export type TriggerConfig =
  | ManualTriggerConfig
  | OnceTriggerConfig
  | RecurringTriggerConfig
  | TaskCreatedTriggerConfig
  | TaskStatusChangedTriggerConfig
  | ConnectorPollTriggerConfig
  | WebhookTriggerConfig

/**
 * Agent type as used in a launchAgent workflow node. A concrete AgentType runs
 * that specific agent; `'fromTask'` defers resolution to run time, reading
 * `task.assignedAgent` from the trigger/queue/taskId context (falling back to
 * `defaults.defaultAgent`). The workflow editor only allows `'fromTask'` when
 * the node actually has a task in scope — see LaunchAgentConfigForm.
 */
export type LaunchAgentType = AiAgentType | 'fromTask'

/**
 * `'fromContext'` defers the boolean to runtime, reading worktree state from
 * the source that launched a contextual workflow (a card or terminal).
 * Editor only allows this sentinel when the trigger is contextual.
 */
export type UseWorktreeOption = boolean | 'fromContext'

// Launch Agent action config
export interface LaunchAgentConfig {
  agentType: LaunchAgentType
  projectName: string
  projectPath: string
  args?: string[]
  displayName?: string
  branch?: string
  useWorktree?: UseWorktreeOption
  worktreeMode?: 'none' | 'new' | 'fromStep' | 'existing'
  worktreeFromStepSlug?: string
  existingWorktreePath?: string
  remoteHostId?: string
  prompt?: string
  promptDelayMs?: number
  taskId?: string
  taskFromQueue?: boolean
  headless?: boolean
  /**
   * JSON Schema the agent's final answer must satisfy. When set (headless only),
   * the engine instructs the agent to emit a matching JSON object, parses it out
   * of the run logs, and stores it as the node's `structuredOutput` — surfaced as
   * typed step vars (`{{steps.<slug>.<field>}}`) that downstream `condition`
   * nodes can gate on instead of substring-matching the model's prose. A run
   * whose output can't be parsed/validated is marked `error`.
   */
  outputSchema?: Record<string, unknown>
  /**
   * How long a headless step may run before the engine gives up on it, in
   * milliseconds. On expiry the agent is killed, the node is marked `error`,
   * and the run finishes — so one agent that never exits can't hold its run
   * open forever. Unset falls back to `defaults.headlessStepTimeoutMinutes`.
   */
  timeoutMs?: number
}

export interface ScriptConfig {
  scriptType: 'bash' | 'powershell' | 'python' | 'node'
  scriptContent: string
  cwd?: string
  projectName?: string // for resolving cwd
  projectPath?: string
  args?: string[]
  /** Caller-supplied id to correlate streaming chunks back to a workflow step.
   *  When set, the runner emits SCRIPT_DATA/SCRIPT_EXIT with this id so the
   *  renderer can show output live in Run History. */
  runId?: string
  /**
   * A connection whose secrets this step runs with, named rather than carried.
   *
   * The id is all the definition holds; the values are read from the server's
   * decrypted store at spawn time and reach this one child's environment only.
   * A workflow that needs a key to talk to a service therefore stays a file
   * anyone can read.
   */
  secretsFrom?: string
}

export type ConditionOperator =
  | 'equals'
  | 'notEquals'
  | 'contains'
  | 'notContains'
  | 'isEmpty'
  | 'isNotEmpty'

export interface ConditionConfig {
  variable: string
  operator: ConditionOperator
  value: string
}

export interface ApprovalConfig {
  message?: string
  timeoutMs?: number
}

/**
 * Upsert a task from `context.connectorItem`. Used as the default downstream
 * of a `connectorPoll` trigger — creates a new task on first sight, updates
 * upstream-owned fields on re-sync. Field ownership: upstream owns
 * title/description; local owns status/assignedAgent/sessionId.
 */
export interface CreateTaskFromItemConfig {
  nodeType: 'createTaskFromItem'
  /** Project the task lands in. `'fromConnection'` = use the connection's
   *  executionProject (or its name as a fallback). */
  project: 'fromConnection' | string
  /** Status for newly-created tasks. Re-syncs never overwrite local status. */
  initialStatus: TaskStatus
}

/**
 * Invoke a manifest-declared connector action (createIssue, closeIssue,
 * commentOnIssue, etc.) with template-rendered args against the connection's
 * stored auth. Template variables like `{{task.title}}` or
 * `{{connectorItem.externalId}}` are resolved from the execution context.
 */
export interface CallConnectorActionConfig {
  nodeType: 'callConnectorAction'
  connectionId: string
  /** Action type from manifest.actions[].type — e.g. 'commentOnIssue'. */
  action: string
  /** The action's authored label, kept so a card can name it without asking the connector. */
  actionLabel?: string
  // Which connector the action belongs to, for a step placed before any connection existed.
  connectorId?: string
  /** Raw args map; values support template placeholders. */
  args: Record<string, string>
}

export interface HttpRequestConfig {
  nodeType: 'httpRequest'
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  /** Absolute, or a path resolved against the profile's base URL. */
  url: string
  /** Header names are literal; values support template placeholders. */
  headers: Record<string, string>
  /** Raw request body; empty sends none. Template placeholders allowed. */
  body: string
  /** An `http` connection whose auth injection is applied server-side. */
  profileConnectionId?: string
}

export type WorkflowNodeConfig =
  | TriggerConfig
  | LaunchAgentConfig
  | ScriptConfig
  | ConditionConfig
  | ApprovalConfig
  | CreateTaskFromItemConfig
  | CallConnectorActionConfig
  | HttpRequestConfig
  | LoopConfig

/**
 * Repeat a run of steps until they are good enough, or until the budget runs
 * out.
 *
 * A loop node sits in the chain ahead of the steps it owns and drives them
 * itself, so the ordinary wave scheduler never runs a body node directly — it
 * only sees them already complete once the loop finishes. That keeps the graph
 * acyclic, which matters because the runner requires every predecessor to have
 * completed and a back edge would simply deadlock.
 *
 * `maxIterations` is not a safety net, it is the contract: an LLM judge asked
 * "is this good yet" converges on yes, so the bound is what actually ends the
 * loop. It is capped low deliberately.
 */
export interface LoopConfig {
  nodeType: 'loop'
  /** Steps this loop owns, in execution order. */
  bodyNodeIds: string[]
  /** Hard cap on passes. 1 means "run the body once", i.e. no repeat. */
  maxIterations: number
  /**
   * Checked after each pass; the loop stops when it holds. Omit to always run
   * exactly `maxIterations` passes. Resolved against step outputs, so
   * `{{steps.review.approved}} equals true` is the shape this exists for.
   */
  until?: ConditionConfig
}

/**
 * What the run does when a node fails.
 *
 * `stop` ends the run and marks everything downstream skipped; `continue` lets
 * successors run anyway, which is what you want for a step whose failure is
 * informational — a notification that didn't send should not sink the work it
 * was reporting on.
 *
 * Absent means `stop`. That is the safer reading of a failure, and it is what
 * a graph drawn as a sequence implies: a step that prepares a branch, or
 * fetches the input the next step parses, has nothing useful to hand on when
 * it fails, and running the rest against missing state produces damage that
 * looks like output.
 */
export type WorkflowNodeErrorPolicy = 'stop' | 'continue'

export interface WorkflowNode {
  id: string
  type: WorkflowNodeType
  label: string
  slug?: string
  config: WorkflowNodeConfig
  position: WorkflowNodePosition
  /** Defaults to `stop` when absent. */
  onError?: WorkflowNodeErrorPolicy
}

export interface WorkflowEdge {
  id: string
  source: string
  target: string
  conditionBranch?: 'true' | 'false'
}

// Execution tracking (runtime only)
export type NodeExecutionStatus =
  | 'pending'
  | 'running'
  | 'success'
  | 'error'
  | 'skipped'
  | 'waiting'

export interface NodeExecutionState {
  nodeId: string
  status: NodeExecutionStatus
  /** Why a skipped node was skipped: a condition branch or a partial run's target slice. */
  skipReason?: 'branch' | 'target'
  startedAt?: string
  completedAt?: string
  sessionId?: string
  error?: string
  logs?: string
  output?: string
  /**
   * Typed payload from a step that declared a shape: a callConnectorAction's
   * `ActionResult.output`, or a headless launchAgent's `outputSchema` result.
   * Stored separately from the string `output` / `logs` fields so the template
   * resolver can walk nested paths like `{{steps.create_issue.html_url}}` at
   * its original shape.
   */
  structuredOutput?: Record<string, unknown>
  /**
   * Which pass of an enclosing loop produced this state. Absent outside a
   * loop. The state itself is last-write-wins, so this reports how many passes
   * ran rather than indexing a history.
   */
  iteration?: number
  taskId?: string
  agentSessionId?: string
  /** Concrete agent type resolved at launch time. Distinct from the node's
   *  configured agentType, which may be the 'fromTask' sentinel. */
  agentType?: AiAgentType
  /** Project name captured at launch so resume works for task-agnostic nodes. */
  projectName?: string
  /** Project path captured at launch. */
  projectPath?: string
  worktreePath?: string
  worktreeName?: string
  /**
   * Whether the worktree at `worktreePath` was created by this node
   * (`'created'`) or inherited from a contextual source like a card / terminal
   * (`'inherited'`). Cleanup pass only removes `'created'` worktrees so a
   * contextual workflow never deletes the parent card's worktree.
   */
  worktreeOrigin?: 'created' | 'inherited'
  /** Timestamp when an approval gate was approved. */
  approvedAt?: string
  /**
   * What the engine did on this step's behalf, and when — what it launched,
   * whether the agent ever produced anything, and how the step ended.
   *
   * Kept out of `logs` on purpose. `logs` is the agent's own stdout/stderr, and
   * a typed step parses it for the declared JSON payload; mixing engine notes
   * into it would both corrupt that parse and misreport what the agent said.
   *
   * This is the answer to "it hung and the output was empty, now what?" — an
   * empty log with a full timeline still tells you whether the process was
   * spawned and whether it ever wrote a byte.
   */
  diagnostics?: string
}

export interface WorkflowDefinition {
  id: string
  name: string
  icon: string
  iconColor: string
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  enabled: boolean
  lastRunAt?: string
  lastRunStatus?: 'success' | 'error'
  staggerDelayMs?: number
  workspaceId?: string // defaults to 'personal' if absent
  autoCleanupWorktrees?: boolean
}

export interface WorkflowExecution {
  /**
   * Identity of this run, unique across every run of every workflow. A workflow
   * can have several runs in flight at once (connector fan-out gives one run per
   * item), so `workflowId` alone does not identify a run.
   *
   * Runs written before this field existed are keyed `<workflowId>:<startedAt>`;
   * that remains the fallback when reading them back, so old history still loads.
   */
  runId: string
  workflowId: string
  startedAt: string
  completedAt?: string
  status: 'running' | 'success' | 'error' | 'cancelled'
  nodeStates: NodeExecutionState[]
  triggerTaskId?: string
  /**
   * What this run was triggered *with* — a connector item id, a task id, or
   * `'manual'`. Two runs of one workflow are duplicates only when this matches
   * as well, which is what lets fan-out run in parallel while a double-fire
   * (two app instances, one scheduler tick) collapses to a single run.
   */
  dedupeParams?: string
  /**
   * Values this run was started with, keyed by `WorkflowInputDef.key`. Kept on
   * the run (not just the live context) so Run History can show what a run was
   * launched with long after it finished.
   */
  inputs?: Record<string, unknown>
  /** Connector payload retained so approval-gated runs can resume downstream steps. */
  connectorItem?: ConnectorItemContext
  /** Durable connector event acknowledged only when this run finishes. */
  connectorInboxId?: number
  /** Ownership token for the connector inbox lease. */
  connectorInboxLeaseToken?: string
  /** Durable terminal handling for restart recovery. */
  connectorInboxDisposition?: 'processed' | 'retry'
  /** True when the run executed only a target step and its upstream slice. */
  partial?: boolean
  /** The failed run this one resumed, reusing its completed step outputs. */
  retryOfRunId?: string
}

/** Stable identity for a run row, tolerating history written before `runId`. */
export function workflowRunId(execution: {
  runId?: string
  workflowId: string
  startedAt: string
}): string {
  return execution.runId || `${execution.workflowId}:${execution.startedAt}`
}

// ─── Tailscale Network Access ────────────────────────────────────

export interface TailscalePeer {
  ip: string
  hostname: string
  dnsName: string
  os: string
  online: boolean
}

/** Where the web client can be reached, independent of Tailscale. */
export interface ReachableUrls {
  urls: string[]
  port: number
  remote: boolean
}

export interface TailscaleStatus {
  installed: boolean
  running: boolean
  backendState: string
  selfIP: string
  selfDNSName: string
  selfOS?: string
  peers: TailscalePeer[]
  appUrl?: string
}

export interface NotificationConfig {
  enabled: boolean
  onWaiting: boolean
  onError: boolean
  onBell: boolean
  soundEnabled?: boolean
  soundVolume?: number // 0.0 – 1.0, default 0.5
}

/**
 * Where the updater currently is, as one value rather than a set of booleans
 * that can contradict each other. The renderer holds the whole object and
 * switches on `kind`.
 *
 * `unsupported` is the dev build: UpdateManager.init() returns early when the
 * app is not packaged, so no event will ever arrive and the panel has to say
 * so rather than sit forever on "checking".
 */
export type UpdateStatus =
  | { kind: 'idle'; lastCheckedAt: number | null }
  | { kind: 'checking' }
  | { kind: 'available'; version: string }
  | { kind: 'downloading'; version: string; percent: number }
  | { kind: 'ready'; version: string }
  | { kind: 'error'; message: string }
  | { kind: 'unsupported' }

export interface AppConfig {
  version: number
  /**
   * Which save produced this snapshot.
   *
   * Round-tripped by the client — every call site spreads the whole config, so it
   * returns untouched — and used by the server to tell a row the client deleted
   * from one it never saw. Absent from a caller that does not track it.
   */
  revision?: number
  defaults: {
    shell: string
    fontSize: number
    theme: 'dark' | 'light'
    rowHeight?: number
    defaultAgent?: AiAgentType
    notifications?: NotificationConfig
    hasSeenOnboarding?: boolean | number
    reopenSessions?: boolean
    // Open Vorn at sign-in. Off until asked; a no-op on Linux.
    startAtLogin?: boolean
    widgetEnabled?: boolean
    taskViewMode?: TaskViewMode
    layoutMode?: 'grid' | 'tabs'
    minimizedPlacement?: MinimizedPlacement
    mainViewMode?: MainViewMode
    activeWorkspace?: string
    updateChannel?: 'stable' | 'beta'
    /**
     * Whether a found update downloads on its own. Install always waits for a
     * restart either way; this only governs the transfer, which matters on a
     * metered connection. Defaults to true — the behaviour before it existed.
     */
    updateAutoDownload?: boolean
    webAccessEnabled?: boolean
    mobileAccessEnabled?: boolean
    networkAccessEnabled?: boolean
    /**
     * The port the server listens on, remembered across restarts.
     *
     * Not cosmetic: a browser keys `localStorage` by origin, so an ephemeral port
     * means a new origin every launch and a paired device loses the token it was
     * given. Chosen on first run and kept; if it is taken at startup the server
     * takes another and remembers that one instead.
     */
    serverPort?: number
    showHeadlessAgents?: boolean
    headlessRetentionMinutes?: number
    /**
     * Ceiling on a headless workflow step, in minutes, for steps that don't set
     * their own timeout. Guards against an agent that starts but never exits —
     * without it the step waits forever and its run never closes. 0 disables it.
     */
    headlessStepTimeoutMinutes?: number
    /**
     * Environment variable names to forward to agent sessions and workflow
     * script nodes even though they match a sensitive prefix. Opt-in and
     * deliberate: the prefix rule stays the default for everything not named
     * here, and nothing else — plain shells, git, tailscale, the detectors —
     * receives them.
     *
     * Whole names, not prefixes: naming ANTHROPIC_API_KEY forwards that one
     * variable and not ANTHROPIC_API_SECRET. Entries are trimmed and compared
     * case-insensitively, so `anthropic_api_key` also matches
     * ANTHROPIC_API_KEY — convenient for a hand-edited config, but worth
     * knowing on POSIX where the two really are different variables.
     */
    envPassthrough?: string[]
    enableHoverPreview?: boolean
    /**
     * Shell sessions only. Replaces the shell's own prompt with a single
     * glyph, so each command reads as a heading above its output instead of
     * repeating your username, host and path on every line. Defaults to on;
     * turn it off to keep your own prompt exactly as your shell renders it.
     */
    minimalShellPrompt?: boolean
    /**
     * Draw finished commands as real elements instead of leaving them in the
     * terminal grid. The live command stays in the terminal; everything
     * already finished becomes a container that can have padding, a boundary
     * and its own copy button without anything being printed into the shell.
     */
    domBlockRendering?: boolean
    /**
     * Whether the server keeps running after the last window closes.
     *
     * The agents are the point. A PTY belongs to the server process, so while
     * the server was a child of the app, quitting killed every session and
     * reopening could only relaunch them from their transcripts — losing the
     * process, the turn in flight, and any prompt waiting for an answer.
     * With this on, quitting closes a window and nothing else; the next launch
     * reconnects to the sessions that were already running.
     *
     * The background server shuts itself down once nothing is left running,
     * so this does not leave a process behind for ever.
     *
     * Defaults to on. Turning it off restores the old behaviour exactly, and
     * "Stop Sessions and Server" in the File menu does it once without changing
     * the setting.
     */
    keepSessionsRunning?: boolean
    /**
     * Set to `true` after the seeded "Default Task Workflow" has been inserted
     * once. Ensures deleting the workflow sticks — we don't resurrect it on
     * the next launch.
     */
    hasSeededDefaultTaskWorkflow?: boolean
    /** Which worktrees the manager treats as stale, and what counts as build output. */
    worktreeRetention?: WorktreeRetentionConfig
  }
  projects: ProjectConfig[]
  agentCommands?: Partial<Record<AiAgentType, AgentCommandConfig>>
  workflows?: WorkflowDefinition[]
  remoteHosts?: RemoteHost[]
  tasks?: TaskConfig[]
  workspaces?: WorkspaceConfig[]
}

export interface RecentSession {
  sessionId: string
  agentType: AiAgentType
  display: string
  projectPath: string
  timestamp: number
  activityCount: number
  activityLabel: string
  canResumeExact: boolean
}

export interface CreateTerminalPayload {
  agentType: AiAgentType
  projectName: string
  projectPath: string
  resumeSessionId?: string
  /** Pre-generated agent session ID to pin on fresh launch (claude, copilot) */
  sessionId?: string
  displayName?: string
  branch?: string
  useWorktree?: boolean
  /** Pass an existing worktree path to reuse it (skips createWorktree) */
  existingWorktreePath?: string
  /** Friendly worktree name (e.g. "amber-aurora") */
  worktreeName?: string
  remoteHostId?: string
  initialPrompt?: string
  promptDelayMs?: number
  headless?: boolean
  /** Workflow metadata — for tagging headless sessions launched by workflows */
  workflowId?: string
  workflowName?: string
  /** Per-invocation arg overrides (replaces settings-level args when set) */
  args?: string[]
  /** Transient: decrypted private key content for stored-key auth. Never persisted. */
  _decryptedKeyContent?: string
  /** Transient: decrypted password for password auth. Never persisted. */
  _decryptedPassword?: string
}

export interface HeadlessSession {
  id: string
  pid: number
  agentType: AiAgentType
  projectName: string
  projectPath: string
  displayName?: string
  branch?: string
  worktreePath?: string
  worktreeName?: string
  isWorktree?: boolean
  status: 'running' | 'exited'
  exitCode?: number
  startedAt: number
  endedAt?: number
  /** Workflow that launched this session */
  workflowId?: string
  workflowName?: string
  /** The agent's own session id (pinned via --session-id for claude/copilot),
   *  enabling later --resume. Only set for agents that support pinning. */
  agentSessionId?: string
  /**
   * Exactly what was spawned, for diagnosing a session that produces nothing.
   * Safe to display: every agent now takes its prompt on stdin, so the argv
   * carries flags and ids only.
   */
  launchCommand?: string
}

export interface ResizePayload {
  id: string
  cols: number
  rows: number
}

export interface FileEntry {
  name: string
  path: string
  isDirectory: boolean
}

export interface GitDiffStat {
  filesChanged: number
  insertions: number
  deletions: number
}

export interface GitFileDiff {
  filePath: string
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  insertions: number
  deletions: number
  diff: string
}

export interface GitDiffResult {
  stat: GitDiffStat
  files: GitFileDiff[]
}

/** Two commits rather than the working tree against HEAD. */
export interface GitDiffRange {
  cwd: string
  from: string
  to: string
}

export interface GitCommitPayload {
  cwd: string
  message: string
  includeUnstaged: boolean
}

export interface GitCommitResult {
  success: boolean
  error?: string
}

/** A shell found on this machine, and what it can report about commands. */
export interface InstalledShell {
  family: 'zsh' | 'bash' | 'fish' | 'powershell' | 'cmd'
  /** Display name, e.g. "PowerShell 7". */
  name: string
  path: string
  version: string | null
  blocks: {
    /**
     * How completely this shell can describe a command.
     *  full    — boundaries, exit status and command text
     *  partial — all of it, but only once the command has finished
     *  limited — boundaries only
     */
    level: 'full' | 'partial' | 'limited'
    /** What it cannot do, phrased for a person rather than a protocol. */
    limitation: string | null
  }
}

/**
 * The safest action available for a worktree, computed server-side so the
 * settings panel, the command palette and any future nudge all agree on what
 * can be touched.
 *
 *  keep    — do nothing; the main worktree, or sessions are running in it
 *  review  — needs a human decision; uncommitted changes, or work that was
 *            never pushed and so cannot be recovered after removal
 *  reclaim — the build artifacts can go but the worktree should stay
 *  remove  — the whole worktree can go; merged and clean
 *  orphan  — a directory git no longer knows about; only `fs.rm` can clear it
 */
export type WorktreeVerdictLevel = 'keep' | 'review' | 'reclaim' | 'remove' | 'orphan'

export interface WorktreeVerdict {
  level: WorktreeVerdictLevel
  /** Bytes recovered by taking the action this level names. */
  freesBytes: number
  /** Short phrases explaining the level, e.g. ['merged into main', 'idle 92 days']. */
  reasons: string[]
  /**
   * Whether "Select suggested" ticks this row. A merged, clean worktree is
   * always removable, but one touched yesterday shouldn't be pre-selected.
   */
  autoSelect: boolean
}

export interface WorktreeInventoryEntry {
  path: string
  name: string
  projectPath: string
  projectName: string
  /** registered — git knows it; orphan-dir — on disk only. */
  kind: 'registered' | 'orphan-dir'
  branch: string | null
  isMain: boolean

  sizeBytes: number
  /** node_modules, dist, out and friends — the part that a reinstall rebuilds. */
  artifactBytes: number
  /** False when sizing timed out or the platform has no fast path; sizes read 0. */
  sizeMeasured: boolean

  lastCommitAt: string | null
  /** Newest git activity in the worktree, from the index mtime. */
  lastTouchedAt: string | null
  idleDays: number | null

  isDirty: boolean
  isMerged: boolean
  hasUpstream: boolean
  activeSessionIds: string[]

  verdict: WorktreeVerdict
}

/** A branch left behind by a removed worktree. */
export interface StaleBranch {
  name: string
  isMerged: boolean
  hasUpstream: boolean
  lastCommitAt: string | null
}

export interface WorktreeProjectInventory {
  projectPath: string
  projectName: string
  defaultBranch: string | null
  /** Set when the project lives on a remote host — sizes come over SSH. */
  remoteHostId: string | null
  entries: WorktreeInventoryEntry[]
  staleBranches: StaleBranch[]
  /** Populated when the project could not be scanned at all. */
  error?: string
}

export interface WorktreeInventory {
  projects: WorktreeProjectInventory[]
  scannedAt: string
}

export interface WorktreeActionFailure {
  path: string
  error: string
}

export interface WorktreeActionResult {
  succeeded: string[]
  failed: WorktreeActionFailure[]
  freedBytes: number
  /** Branches deleted alongside removed worktrees. */
  deletedBranches: string[]
}

export interface BranchDeleteResult {
  deleted: string[]
  failed: { branch: string; error: string }[]
}

/** Retention preferences for the worktree manager. */
export interface WorktreeRetentionConfig {
  /**
   * A merged, clean worktree idle for at least this many days is pre-selected
   * by "Select suggested". 0 pre-selects every removable worktree.
   */
  idleDaysThreshold?: number
  /** Directory names treated as rebuildable build output. */
  artifactDirs?: string[]
  /** Worktree paths the scanner always reports as `keep`. */
  pinnedPaths?: string[]
}

export const DEFAULT_ARTIFACT_DIRS = [
  'node_modules',
  'dist',
  'out',
  '.next',
  '.turbo',
  '.nuxt',
  'target',
  'coverage',
  '.venv',
  '__pycache__'
]

export const DEFAULT_IDLE_DAYS_THRESHOLD = 14

export const IPC = {
  TERMINAL_CREATE: 'terminal:create',
  TERMINAL_WRITE: 'terminal:write',
  TERMINAL_RESIZE: 'terminal:resize',
  TERMINAL_KILL: 'terminal:kill',
  TERMINAL_ATTACH: 'terminal:attach',
  TERMINAL_LIST_ACTIVE: 'terminal:listActive',
  TERMINAL_DATA: 'terminal:data',
  TERMINAL_BELL: 'terminal:bell',
  TERMINAL_EXIT: 'terminal:exit',
  SESSION_CREATED: 'session:created',
  SESSION_UPDATED: 'session:updated',
  SESSION_REORDERED: 'session:reordered',
  TERMINAL_RENAME: 'terminal:rename-session',
  TERMINAL_REORDER: 'terminal:reorder-sessions',
  CONFIG_LOAD: 'config:load',
  CONFIG_SAVE: 'config:save',
  CONFIG_CHANGED: 'config:changed',
  SESSIONS_RESTORED: 'sessions:restored',
  SESSIONS_RESUME: 'sessions:resume',
  SESSIONS_CLEAR: 'sessions:clear',
  SESSIONS_GET_RECENT: 'sessions:getRecent',
  DIALOG_OPEN_DIRECTORY: 'dialog:openDirectory',
  IDE_DETECT: 'ide:detect',
  PROJECT_DETECT_MOBILE: 'project:detectMobile',
  IDE_OPEN: 'ide:open',
  GIT_IS_REPO: 'git:isGitRepo',
  GIT_LIST_BRANCHES: 'git:listBranches',
  GIT_LIST_REMOTE_BRANCHES: 'git:listRemoteBranches',
  GIT_CREATE_WORKTREE: 'git:createWorktree',
  GIT_REMOVE_WORKTREE: 'git:removeWorktree',
  GIT_RENAME_WORKTREE_BRANCH: 'git:renameWorktreeBranch',
  GIT_RENAME_WORKTREE: 'git:renameWorktree',
  GIT_WORKTREE_DIRTY: 'git:worktreeDirty',
  GIT_LIST_WORKTREES: 'git:listWorktrees',
  GIT_CHECKOUT_BRANCH: 'git:checkoutBranch',
  GIT_GET_WORKTREE_BRANCH: 'git:getWorktreeBranch',
  WORKTREE_CONFIRM_CLEANUP: 'worktree:confirmCleanup',
  WORKTREE_ACTIVE_SESSIONS: 'worktree:activeSessions',
  WORKTREE_INVENTORY: 'worktree:inventory',
  WORKTREE_RECLAIM_ARTIFACTS: 'worktree:reclaimArtifacts',
  WORKTREE_REMOVE_MANY: 'worktree:removeMany',
  WORKTREE_PRUNE_ORPHANS: 'worktree:pruneOrphans',
  GIT_DELETE_BRANCHES: 'git:deleteBranches',
  GIT_GET_BRANCH: 'git:getBranch',
  GIT_DIFF_STAT: 'git:diffStat',
  GIT_DIFF_FULL: 'git:diffFull',
  GIT_COMMIT: 'git:commit',
  GIT_PUSH: 'git:push',
  DIALOG_OPEN_FILE: 'dialog:openFile',
  SCHEDULER_EXECUTE: 'scheduler:execute',
  SCHEDULER_STOP_RUN: 'scheduler:stopRun',
  SCHEDULER_MISSED: 'scheduler:missed',
  SCHEDULER_GET_LOG: 'scheduler:getLog',
  SCHEDULER_GET_NEXT_RUN: 'scheduler:getNextRun',
  WORKFLOW_EXECUTION_COMPLETE: 'workflow:executionComplete',
  WORKFLOW_GATE_RESOLVED: 'workflow:gateResolved',
  WINDOW_MINIMIZE: 'window:minimize',
  WINDOW_MAXIMIZE: 'window:maximize',
  WINDOW_CLOSE: 'window:close',
  WINDOW_IS_MAXIMIZED: 'window:isMaximized',
  WINDOW_MAXIMIZED_CHANGED: 'window:maximizedChanged',
  WIDGET_STATUS_UPDATE: 'widget:status-update',
  WIDGET_FOCUS_TERMINAL: 'widget:focus-terminal',
  WIDGET_HIDE: 'widget:hide',
  WIDGET_TOGGLE: 'widget:toggle',
  WIDGET_RENDERER_STATUS: 'widget:renderer-status',
  WIDGET_SET_ENABLED: 'widget:set-enabled',
  WIDGET_PERMISSION_REQUEST: 'widget:permission-request',
  WIDGET_PERMISSION_RESPONSE: 'widget:permission-response',
  WIDGET_PERMISSION_CANCELLED: 'widget:permission-cancelled',
  SHELL_CREATE: 'shell:create',
  /** Renderer reports a browser guest's webContentsId once it attaches, which
   *  is the only thread tying a `<webview>` back to the session that owns it. */
  BROWSER_ATTACH: 'browser:attach',
  BROWSER_DETACH: 'browser:detach',
  /** Renderer names the design file its pane is showing, so main can watch it.
   *  Null stops watching. The renderer decides because it is the side that read
   *  the manifest and knows the page is a design. */
  BROWSER_WATCH_FILE: 'browser:watchFile',
  /** Main tells the renderer a watched design changed on disk. */
  BROWSER_FILE_CHANGED: 'browser:fileChanged',
  /** Renderer asks what the loaded page declares itself to be, so the pane can
   *  draw artifact chrome instead of an address bar. Only main can ask the
   *  guest — the renderer has no CDP. */
  BROWSER_READ_MANIFEST: 'browser:readManifest',
  /** Renderer writes one declared tweak value into the page. */
  BROWSER_SET_TWEAK: 'browser:setTweak',
  /** Renderer arms the element picker; main pushes the result back on pick. */
  BROWSER_PICK_START: 'browser:pickStart',
  BROWSER_PICK_CANCEL: 'browser:pickCancel',
  BROWSER_PICKED: 'browser:picked',
  /** Freehand ink over the page, resolved to the elements it covers. */
  BROWSER_ANNOTATE: 'browser:annotate',
  /** Main asks the renderer to open (or retarget) a session's browser pane.
   *  The pane lives in renderer state, so main cannot create one itself. */
  BROWSER_OPEN_PANE: 'browser:openPane',
  /** Main asks the renderer to add, close, or switch a tab in that pane. */
  BROWSER_TAB_COMMAND: 'browser:tabCommand',
  /** Renderer reports what its tab strip holds, after any change to it. The
   *  strip is renderer state, so this is the only way main can answer a
   *  listing without keeping a second copy that would drift. */
  BROWSER_TABS_CHANGED: 'browser:tabsChanged',
  /** Main asks the renderer to open a session's device pane. As with the
   *  browser, the pane is renderer state, so main can only ask. */
  DEVICE_OPEN_PANE: 'device:openPane',
  /** The pane's own poll and tap-through. A simulator has no `<webview>`, so
   *  unlike the browser pane there is nothing the renderer can drive directly —
   *  every frame and every touch goes through main. */
  DEVICE_SCREENSHOT: 'device:screenshot',
  DEVICE_INTERACT: 'device:interact',
  DEVICE_LIST: 'device:list',
  DEVICE_CLAIM: 'device:claim',
  DEVICE_RELEASE: 'device:release',
  /**
   * Resolve a point the person picked on the device pane to the element there.
   *
   * Unlike the browser's picker there is nothing to arm in main: the pane is
   * showing a still it already has, so arming and highlighting are purely
   * renderer-local and only the point→element lookup needs the companion.
   */
  DEVICE_PICKED: 'device:picked',
  /** Freehand ink over the device screen, resolved to the elements beneath. */
  DEVICE_ANNOTATE: 'device:annotate',
  UPDATE_INSTALL: 'update:install',
  UPDATE_SET_CHANNEL: 'update:set-channel',
  /** Pushed on every updater transition, so the renderer holds one value. */
  UPDATE_STATUS: 'update:status',
  /** Manual check, from the Updates settings panel. */
  UPDATE_CHECK: 'update:check',
  /** Start a transfer the user deferred by turning auto-download off. */
  UPDATE_DOWNLOAD: 'update:download',
  /** Mirrors the config toggle onto the live updater, no restart needed. */
  UPDATE_SET_AUTO_DOWNLOAD: 'update:set-auto-download',
  /** Synchronous read, so a freshly-opened panel renders without waiting. */
  UPDATE_GET_STATUS: 'update:get-status',
  TASK_IMAGE_SAVE: 'task:imageSave',
  TASK_IMAGE_DELETE: 'task:imageDelete',
  TASK_IMAGE_GET_PATH: 'task:imageGetPath',
  TASK_IMAGE_CLEANUP: 'task:imageCleanup',
  DIALOG_OPEN_IMAGE: 'dialog:openImage',
  DIALOG_SAVE_TEXT_FILE: 'dialog:saveTextFile',
  HEADLESS_CREATE: 'headless:create',
  HEADLESS_KILL: 'headless:kill',
  HEADLESS_LIST: 'headless:list',
  HEADLESS_DATA: 'headless:data',
  HEADLESS_EXIT: 'headless:exit',
  SCRIPT_EXECUTE: 'script:execute',
  SCRIPT_DATA: 'script:data',
  SCRIPT_EXIT: 'script:exit',
  WORKFLOW_RUN_SAVE: 'workflowRun:save',
  WORKFLOW_RUN_LIST: 'workflowRun:list',
  WORKFLOW_RUN_LIST_BY_TASK: 'workflowRun:listByTask',
  WORKFLOW_RUN_LIST_WAITING: 'workflowRun:listWaiting',
  WORKFLOW_RUN_LIST_RUNNING: 'workflowRun:listRunning',
  WORKFLOW_RUN_LIST_ALL: 'workflowRun:listAll',
  WORKFLOW_RUN_CLAIM: 'workflowRun:claim',
  WORKFLOW_RUN_RELEASE: 'workflowRun:release',
  SESSION_EVENT_LIST: 'sessionEvent:list',
  SESSION_EVENT_LIST_BY_SESSION: 'sessionEvent:listBySession',
  AGENT_DETECT_INSTALLED: 'agent:detectInstalled',
  TAILSCALE_STATUS: 'tailscale:status',
  SERVER_REACHABLE_URLS: 'server:reachableUrls',
  PAIRING_START: 'pairing:start',
  PAIRING_APPROVE: 'pairing:approve',
  PAIRING_DENY: 'pairing:deny',
  PAIRING_CANCEL: 'pairing:cancel',
  PAIRING_PENDING: 'pairing:pending',
  PAIRING_REQUESTED: 'pairing:requested',
  PAIRING_COLLECTED: 'pairing:collected',
  TOKEN_LIST: 'token:list',
  TOKEN_CREATE: 'token:create',
  TOKEN_REVOKE: 'token:revoke',
  CREDENTIAL_STORE_KEY: 'credential:storeKey',
  CREDENTIAL_IMPORT_KEY_FILE: 'credential:importKeyFile',
  CREDENTIAL_DELETE_KEY: 'credential:deleteKey',
  CREDENTIAL_LIST_KEYS: 'credential:listKeys',
  CREDENTIAL_GET_ENCRYPTED_KEY: 'credential:getEncryptedKey',
  CREDENTIAL_ENCRYPT: 'credential:encrypt',
  CREDENTIAL_SAFE_STORAGE_AVAILABLE: 'credential:safeStorageAvailable',
  SSH_TEST_CONNECTION: 'ssh:testConnection',
  OPEN_EXTERNAL: 'shell:openExternal',
  FILE_LIST_DIR: 'file:listDir',
  FILE_READ_CONTENT: 'file:readContent',
  FILE_STAMP: 'file:stamp',
  FILE_WRITE_CONTENT: 'file:writeContent',
  SHELL_LIST_EXECUTABLES: 'shell:listExecutables',
  SHELL_LIST_INSTALLED: 'shell:listInstalled',
  CONNECTOR_LIST: 'connector:list',
  CONNECTOR_GET: 'connector:get',
  CONNECTION_LIST: 'connection:list',
  CONNECTION_CREATE: 'connection:create',
  CONNECTION_UPDATE: 'connection:update',
  CONNECTION_DELETE: 'connection:delete',
  CONNECTION_GET_SOURCE_LINK: 'connection:getSourceLink',
  CONNECTOR_DETECT_REPO: 'connector:detectRepo',
  CONNECTOR_SEED_WORKFLOW: 'connector:seedWorkflow',
  CONNECTOR_STATUS: 'connector:status',
  CONNECTOR_PROBE_AUTH: 'connector:probeAuth',
  CONNECTION_UPSERT_FROM_ITEM: 'connection:upsertFromItem',
  CONNECTOR_INBOX_COMPLETE: 'connector:inboxComplete',
  CONNECTOR_INBOX_RENEW: 'connector:inboxRenew',
  WORKFLOW_RUN_MANUAL: 'workflow:runManual',
  CONNECTION_BACKFILL: 'connection:backfill',
  CREDENTIALS_SET_DECRYPTED: 'credentials:setDecrypted',
  CREDENTIALS_CLEAR_DECRYPTED: 'credentials:clearDecrypted',
  CONNECTION_EXECUTE_ACTION: 'connection:executeAction',
  CONNECTION_LIST_ACTIONS: 'connection:listActions',
  WEBHOOK_INFO: 'webhook:info',
  HTTP_REQUEST: 'http:request',
  CONNECTION_PREFLIGHT: 'connection:preflight',
  CONNECTION_LIST_KEYS: 'connection:listKeys',
  CONNECTION_ROTATE_SECRET: 'connection:rotateSecret',
  CONNECTION_LIST_MCP_TOOLS: 'connection:listMcpTools',
  CONNECTION_REFRESH_MCP_TOOLS: 'connection:refreshMcpTools',
  CONNECTOR_PROBE_SDK: 'connector:probeSdk',
  CONNECTOR_CATALOG: 'connector:catalog',
  CONNECTOR_CATALOG_REFRESH: 'connector:catalogRefresh',
  CONNECTOR_INSPECT_PACK: 'connector:inspectPack',
  CONNECTOR_INSTALL_PACK: 'connector:installPack',
  CONNECTOR_REMOVE_PACK: 'connector:removePack',
  CONNECTOR_ROLLBACK_PACK: 'connector:rollbackPack',
  CONNECTOR_LIST_PACKS: 'connector:listPacks',
  CONNECTOR_INSTALL_PROGRESS: 'connector:installProgress'
} as const

/**
 * Self-description read from a connector package built with
 * `@vornrun/connector-sdk`, used to fill in a connection form.
 */
export interface SdkSetupFilters {
  pollTool: string
  itemsPath: string
  idField: string
  timestampField: string
  titleField: string
  urlField: string
  cursorArg: string
  cursorPath: string
}

export interface SdkEnvVar {
  name: string
  required: boolean
  /** Stored via the OS keychain rather than in the config file. */
  secret: boolean
  description?: string
}

export interface SdkTrigger {
  type: string
  label: string
  description?: string
  /** Connection filter values that make this trigger poll correctly. */
  filters: SdkSetupFilters
  /**
   * What an upstream status should become locally. Seeds the connection's own
   * mapping, which the person setting it up then owns. Absent means the
   * connector said nothing, and everything it imports lands as `todo`.
   */
  statusMapping?: Array<{ upstream: string; suggestedLocal: TaskStatus }>
  /**
   * The polling workflow to create with the connection. Without one a
   * connector that fires on a schedule connects and then sits silent until
   * somebody builds the workflow by hand.
   */
  defaultWorkflow?: { name: string; defaultCronFromMinutes: number }
}

/**
 * A connector's own glyph.
 *
 * Path data only, never markup: this arrives from a third-party package and is
 * drawn inside an `<svg>` the renderer owns, so there is nothing for a
 * connector to inject.
 */
export interface SdkConnectorIcon {
  viewBox: string
  paths: string[]
}

/**
 * A connector package Vorn ships knowledge of, so it can be offered by name
 * instead of requiring one.
 *
 * Metadata only. What a connection actually needs is read from the package at
 * install time, so this cannot become a stale second copy of the connector's
 * own definition.
 */
export interface ConnectorCatalogSummary {
  type: string
  label: string
  description?: string
}

/** One choice a `select` argument offers. */
export interface ConnectorCatalogActionOption {
  value: string
  label?: string
}

/** An argument an action takes, carried so a step can be offered before install. */
export interface ConnectorCatalogActionInput {
  key: string
  label: string
  type: string
  required: boolean
  /** Fixed choices, so a `select` can be drawn before anything is installed. */
  options?: ConnectorCatalogActionOption[]
  /** An options set the connector serves, resolved against a live connection. */
  loadOptions?: string
}

/**
 * An action, described well enough to become a step in the library before the
 * connector it belongs to is on disk.
 */
export interface ConnectorCatalogAction extends ConnectorCatalogSummary {
  inputs?: ConnectorCatalogActionInput[]
}

/**
 * What the factory checked, and when.
 *
 * "Verified" is a receipt rather than a word: the checks that ran, against
 * which version, on which date. A catalog that says nothing here is not
 * claiming a connector is bad — only that nothing vouched for it.
 */
export interface ConnectorCatalogVerification {
  /**
   * Which receipt format this is. A build reads only the shape it knows: what
   * "verified" vouches for is exactly the checks that ran, so a later format
   * meaning something else must not be shown under this build's badge.
   */
  schema: 1
  /** The version the checks ran against, which may trail the published one. */
  version: string
  /** ISO timestamp of the last run. */
  checkedAt: string
  /** Names of the checks that passed, e.g. `manifest`, `no-runtime-deps`. */
  checks: string[]
}

export interface ConnectorCatalogEntry {
  id: string
  name: string
  description: string
  /** npm package the connector is published as. */
  packageName: string
  /** Published version, so a listing can say what would be installed. */
  version?: string
  /** Where the installable pack is published, when one is. */
  packUrl?: string
  /** Checksum the download must match, when the catalog publishes one. */
  sha256?: string
  capabilities: Array<'tasks' | 'triggers' | 'actions'>
  /** One line on how it authenticates, shown before anyone commits to install. */
  auth?: string
  /**
   * Which rung that one line describes, so the list can be filtered by what
   * setting a connector up will actually ask of you. Absent on an older
   * catalog, which reads as unknown rather than as none.
   */
  authRung?: ConnectorAuthRung
  /** What the factory checked, when a connector has been through it. */
  verified?: ConnectorCatalogVerification
  icon?: SdkConnectorIcon
  /** Groups the connector in the list once there are too many to scan. */
  category?: string
  /** Extra search terms, so it is findable by what it talks to. */
  keywords?: string[]
  /**
   * What the connector fires on, what a workflow can ask it to do, and what it
   * will want configured — generated upstream from the connector's own
   * manifest, so a listing can answer "will this do what I need" before
   * anything is downloaded. Absent on an older catalog.
   */
  triggers?: ConnectorCatalogSummary[]
  actions?: ConnectorCatalogAction[]
  env?: Array<{ name: string; required: boolean; description?: string }>
}

/**
 * A catalog entry plus where to actually launch it, resolved in the main
 * process because that resolution depends on the filesystem.
 */
export interface ConnectorCatalogItem extends ConnectorCatalogEntry {
  launch: { command: string; args: string[] }
}

/**
 * The catalog plus when it was last fetched.
 *
 * `fetchedAt` is absent when nothing has ever been fetched — a first run, or
 * every attempt so far has failed — so the UI can say that rather than showing
 * a timestamp for a list that may be missing everything published since.
 */
/**
 * A workflow someone can start from rather than an empty canvas.
 *
 * Published beside the connectors and carried in the same document, because a
 * template goes stale for the same reasons a connector entry does and there is
 * no reason to fetch, cache and repair two lists.
 */
export interface WorkflowTemplate {
  id: string
  name: string
  description: string
  /** The chain as a person reads it, e.g. ['Webhook', 'Condition', 'HTTP request']. */
  steps: string[]
  category?: string
  portable: PortableWorkflow
}

/**
 * An MCP server worth knowing about, listed beside the connectors.
 *
 * It carries a command rather than a package because that is what a generic
 * server is: something Vorn starts and speaks MCP to. There is no manifest to
 * probe, so what a person needs is the launch line filled in for them.
 */
export interface McpServerCatalogEntry {
  id: string
  name: string
  description?: string
  command: string
  args: string[]
  category?: string
  keywords?: string[]
  /** Environment variables the server expects, named so the form can ask. */
  env?: string[]
}

export interface ConnectorCatalogSnapshot {
  items: ConnectorCatalogItem[]
  templates: WorkflowTemplate[]
  mcpServers: McpServerCatalogEntry[]
  fetchedAt?: number
}

/**
 * How a connector signs in, lowest rung first — mirrors the SDK's own
 * declaration so the app can say how a connector authenticates before it is
 * installed, and show an identity instead of a token field where one is
 * already signed in.
 */
export type ConnectorAuthRung = 'none' | 'cli' | 'key' | 'oauth'

export interface SdkConnectorAuth {
  rung: ConnectorAuthRung
  /** Asks the borrowed tool who you are. Present for `cli`. */
  probe?: { command: string; args?: string[] }
  // What to take from the signed-in tool at spawn; never stored.
  borrow?: { env?: string[]; tokenArgs?: string[]; tokenEnv?: string }
  /** Config field keys holding the credential. Present for `key`. */
  keys?: string[]
}

/** An argument a packaged connector's action takes. */
export interface SdkActionInput {
  key: string
  label: string
  type: string
  required: boolean
  /** Fixed choices, when the action declared a `select` with known values. */
  options?: Array<{ value: string; label?: string }>
  /** An options set the connector serves, resolved against a live connection. */
  loadOptions?: string
}

/** An action a packaged connector serves, as its manifest describes it. */
export interface SdkAction {
  type: string
  label: string
  description?: string
  /** Absent on a manifest read before inputs were carried through. */
  inputs?: SdkActionInput[]
  outputs?: Array<{ key: string; type?: string; description?: string }>
}

export interface SdkConnectorManifest {
  id: string
  name: string
  version: string
  description?: string
  icon?: SdkConnectorIcon
  /** Absent on a connector built before rungs existed, which reads as unknown. */
  auth?: SdkConnectorAuth
  triggers: SdkTrigger[]
  actions: SdkAction[]
  /** Union of the environment variables the connector reads. */
  env: SdkEnvVar[]
}

/** A connector installed on disk, where `version` is what runs rather than what was asked for. */
export interface InstalledConnectorPack {
  id: string
  name: string
  version: string
  description?: string
  icon?: SdkConnectorIcon
  /** How this connector signs in, read from the manifest that was installed. */
  auth?: SdkConnectorAuth
  /** Directory holding the running version's files. */
  path: string
  /** The one version kept behind the current one, when a rollback is possible. */
  previousVersion?: string
  installedAt: number
  bytes: number
  triggers: SdkTrigger[]
  actions: SdkAction[]
  env: SdkEnvVar[]
}

/** Where a pack is read from; `staged` is one an inspection already verified. */
export type ConnectorPackSource =
  | { kind: 'file'; path: string }
  | { kind: 'url'; url: string; sha256?: string }
  | { kind: 'npm'; packageName: string }
  | { kind: 'staged'; token: string }

export type ConnectorPackResult =
  | { ok: true; pack: InstalledConnectorPack }
  | { ok: false; error: string }

/** What a verified pack says about itself, before any of it is kept. */
export interface ConnectorPackSummary {
  id: string
  name: string
  version: string
  description?: string
  icon?: SdkConnectorIcon
  /** What signing in will ask for, said before any of this is kept. */
  auth?: SdkConnectorAuth
  triggers: SdkTrigger[]
  actions: SdkAction[]
  env: SdkEnvVar[]
  /** The version already on disk, when this would replace one. */
  installedVersion?: string
  /** Handle to the verified files, so confirming installs exactly what was shown. */
  token: string
}

export type ConnectorPackPreview =
  | { ok: true; preview: ConnectorPackSummary }
  | { ok: false; error: string }

/** `id` is the connector's once its manifest is read, and the source label until then. */
export interface ConnectorInstallProgress {
  id: string
  phase: 'checking' | 'downloading' | 'verifying' | 'installing' | 'installed' | 'failed'
  /** Download completion, absent when the size was not advertised. */
  percent?: number
  version?: string
  error?: string
}

export interface SdkProbeRequest {
  command: string
  args: string[]
  /** Non-secret env for the probe; secrets are not needed to read a manifest. */
  env?: Record<string, string>
}

export type SdkProbeResult =
  | { ok: true; manifest: SdkConnectorManifest }
  | { ok: false; error: string }

export interface PermissionSuggestion {
  type: 'addRules' | 'setMode' | string
  destination?: string // "session" | "localSettings"
  behavior?: string // "allow"
  rules?: Array<{ toolName?: string; ruleContent?: string }>
  mode?: string // "acceptEdits" | "plan"
  [key: string]: unknown
}

export interface AskUserQuestion {
  question: string
  header?: string
  multiSelect?: boolean
  options?: Array<{ label: string; description?: string }>
}

export interface HookEvent {
  session_id: string
  hook_event_name: string
  cwd: string
  tool_name?: string
  tool_input?: Record<string, unknown>
  tool_use_id?: string
  permission_mode?: string
  transcript_path?: string
  message?: string
  title?: string
  permission_suggestions?: PermissionSuggestion[]
}

export interface PermissionRequestInfo {
  requestId: string
  sessionId: string
  terminalId?: string
  toolName: string
  toolInput: Record<string, unknown>
  description?: string
  agentType?: AgentType
  projectName?: string
  permissionSuggestions?: PermissionSuggestion[]
  /** Populated when toolName === "AskUserQuestion" */
  questions?: AskUserQuestion[]
}

export interface WidgetAgentInfo {
  id: string
  agentType: AgentType
  displayName?: string
  projectName: string
  status: AgentStatus
}

export interface ScheduleLogEntry {
  workflowId: string
  workflowName: string
  executedAt: string
  status: 'success' | 'error' | 'missed'
  sessionsLaunched: number
  error?: string
}

// ─── Agent-controllable browser pane ────────────────────────────

/**
 * One node of a page as the agent sees it.
 *
 * Derived from the accessibility tree rather than the DOM: the AX tree already
 * collapses presentational wrappers and carries the computed name a user would
 * read, so it is both smaller and closer to what "the page says" than raw HTML.
 */
export interface BrowserNode {
  /** Opaque handle for `interact`. Only interactive nodes carry one. */
  ref?: string
  role: string
  name?: string
  value?: string
  /** Set when the node is not currently actionable, so the agent stops early. */
  disabled?: boolean
}

export interface BrowserPageRead {
  url: string
  title: string
  nodes: BrowserNode[]
  /** Pass back as `cursor` to continue. Absent when the page is exhausted. */
  nextCursor?: string
  /** Bumped on every navigation; refs from an older generation are refused. */
  generation: number
  /**
   * What the page declares itself to be, when it declares anything.
   *
   * Present only for a design artifact. An agent asked to change one needs to
   * know which values it derives from before it starts editing rules.
   */
  artifact?: ArtifactManifest
  /**
   * The values the design is *currently* showing, which are not the defaults
   * written in the file.
   *
   * A person can turn a control without spending an agent turn, so the file
   * says 6000 while the screen says 9000. An agent asked to make the
   * over-budget case louder has to work from the second number.
   */
  artifactValues?: Record<string, unknown>
}

export interface BrowserConsoleMessage {
  level: string
  text: string
  timestamp: number
}

export interface BrowserNetworkRequest {
  method: string
  url: string
  status?: number
  timestamp: number
}

/**
 * The Chromium partition a session's browser guests live in.
 *
 * Shared because two sides must agree exactly: the renderer puts its
 * `<webview>` in this partition, and main installs the filter that bounds what
 * files those guests may read on the same one. Spelled out twice, a rename
 * would leave the filter attached to a partition no guest uses — installing
 * successfully, logging nothing, and enforcing nothing.
 */
export function browserPartition(sessionId: string): string {
  return `persist:vorn-browser-${sessionId}`
}

/**
 * One tab in a session's browser pane, as the agent sees it.
 *
 * `url` is where the guest actually is, not where the tab was originally sent —
 * a tab that redirected or followed a link would otherwise be listed under a
 * page nobody is looking at, and `index` would name it wrongly.
 */
export interface BrowserTabInfo {
  /** Zero-based, and what `browser_tabs` close/select take. */
  index: number
  url: string
  /** The page's own title, when it has reported one. */
  title?: string
  active: boolean
}

/**
 * One adjustable value a design artifact declares.
 *
 * A tweak earns its place when one value drives many places at once, or switches
 * between two treatments. Text and single colours are not tweaks — a person can
 * already edit copy and recolour an element directly, and declaring those would
 * put a control beside every word.
 */
export type ArtifactTweak =
  | {
      type: 'number'
      label?: string
      default: number
      unit?: string
      min?: number
      max?: number
      step?: number
    }
  | { type: 'boolean'; label?: string; default: boolean }
  | { type: 'color'; label?: string; default: string; options?: string[] }
  | { type: 'select'; label?: string; default: string; options: string[] }

/**
 * What a file says it is, read out of its own `<script id="artifact">` block.
 *
 * The block's *presence* is what marks a page as an artifact — not the presence
 * of tweaks. A design with nothing to adjust is still a design, and gating the
 * chrome on tweaks would leave it indistinguishable from an ordinary web page.
 *
 * Authored by the page, so everything here is validated rather than trusted:
 * `parseManifest` in the main process drops anything malformed instead of
 * throwing, because "this is not an artifact" is an ordinary answer.
 */
export interface ArtifactManifest {
  /**
   * What sort of artifact this is. Deliberately one value for now — a second
   * earns its place only when it needs different chrome, and a vocabulary of
   * kinds that all render identically is how `lib/task-status.ts` ended up with
   * five colour maps that disagreed.
   */
  kind: 'design'
  /** Shown in the pane header in place of the address. */
  title?: string
  /** Declared inputs, keyed by name. Absent when the artifact has none. */
  tweaks?: Record<string, ArtifactTweak>
}

/** Where an interaction lands: a ref from `read_page`, or raw viewport coords. */
export type BrowserTarget = { ref: string } | { x: number; y: number }

/**
 * What the user pointed at, packaged for the agent.
 *
 * The picker exists because "this button" is trivial for a person to indicate
 * and expensive for an agent to locate. Everything here answers "which element
 * is it" from a different angle, so a mismatch in one is recoverable.
 */
export interface BrowserSelection {
  /** Where it was: page url and the element's viewport rect. */
  url: string
  rect: { x: number; y: number; width: number; height: number }
  /** What it says. Usually the fastest way to recognise it in a page read. */
  text: string
  /** CSS-ish path from the document root, for a targeted re-lookup. */
  selector: string
  /** The element's own markup, truncated — attributes often carry the intent. */
  outerHTML: string
  /** Tag plus id and classes, split out so they need not be parsed back. */
  tagName: string
  id?: string
  classes?: string[]
  /**
   * React component name, when the page is a dev build carrying fiber data.
   * A bonus: production builds mangle or drop it, so nothing may depend on it.
   */
  componentName?: string
  /** `file:line` from React's `_debugSource`. Dev builds only, never required. */
  source?: string
  /** PNG of just this element, base64. Small by construction — it's one node. */
  screenshot?: string
}

/** One freehand stroke, in the coordinate space of what it was drawn over. */
export interface BrowserStroke {
  points: Array<{ x: number; y: number }>
}

/**
 * Ink over a page, resolved to the things underneath it.
 *
 * Both halves ship because neither is sufficient. The image carries intent that
 * geometry cannot — a circle round three items, an arrow from one to another,
 * a scribble that means "this bit" — while the elements carry identity the
 * image cannot, since a picture of a button is not a handle on it.
 */
export interface BrowserAnnotation {
  url: string
  /** Elements found under the ink, nearest-first, with refs where actionable. */
  elements: BrowserNode[]
  /** Full-page PNG with the ink drawn on, base64. */
  image: string
  /** Tight crop around the strokes' bounding box, base64. Usually the useful one. */
  crop?: string
  /** The ink's bounding box, in page coordinates. */
  bounds: { x: number; y: number; width: number; height: number }
}

// ─── Agent-controllable device pane ─────────────────────────────

/**
 * What a project directory says about whether it is a mobile app.
 *
 * Detection reads declared dependencies and config, never build output. The
 * tempting signal — an `ios/` directory — is wrong: Expo's Continuous Native
 * Generation means a real, shipping Expo app commonly has no `ios/` at all, so
 * gating on it hides the device control precisely where it is most wanted.
 */
export interface MobileProject {
  isMobile: boolean
  framework: 'expo' | 'react-native' | 'flutter' | 'ios-native' | null
  /**
   * True when a simulator cannot run this project straight from source — the
   * managed Expo case, where a dev client (or Expo Go) must be installed first.
   * Booting a device without saying so hands the person a clean iPhone with
   * none of their app on it and no explanation.
   */
  needsDevClient: boolean
}

/** A simulator on this machine, and who currently holds it. */
export interface DeviceInfo {
  udid: string
  name: string
  runtime: string
  booted: boolean
  /** The session holding it. Claiming a held device fails, naming this. */
  claimedBy?: string
}

/**
 * A file as it was when a draft was based on it.
 *
 * Size and mtime, which is what every editor uses for this and what a remote
 * host can answer over one `stat`. Not a hash: the question is whether the file
 * has changed under an unsaved edit, and answering it must not cost reading the
 * whole file every time somebody saves.
 */
export interface FileStamp {
  size: number
  mtimeMs: number
}

/**
 * Why a device could not be claimed.
 *
 * A closed set, because the caller has to act differently on each and a message
 * string cannot be branched on without matching its wording. Re-claiming a
 * device on launch is what forced this: "gone" is ordinary and silent — the
 * simulator was deleted, or the record came from another machine — while "held
 * by another session" is worth saying out loud, and the two used to arrive as
 * prose that read the same.
 */
export type DeviceClaimFailure =
  /** No simulator with that udid on this machine any more. */
  | { reason: 'gone'; message: string }
  /** Another session in this Vorn holds it. `holder` is that session's id. */
  | { reason: 'held-by-session'; message: string; holder: string }
  /** Another Vorn process holds it. `pid` is that process. */
  | { reason: 'held-by-other-vorn'; message: string; pid: number }
  /** The simulator exists and is free, but would not boot. */
  | { reason: 'boot-failed'; message: string }

export type DeviceClaimResult =
  | { ok: true; udid: string; name: string; booted: boolean }
  | ({ ok: false } & DeviceClaimFailure)

/** A point in **points**, the accessibility tree's units — never pixels. */
export interface DevicePoint {
  x: number
  y: number
}

/**
 * One element of a screen as the agent sees it.
 *
 * The mobile counterpart of `BrowserNode`, with one structural difference worth
 * knowing: a `ref` here resolves to a *coordinate that was correct when the
 * screen was read*, not to a node identity. Nothing detects the screen moving
 * underneath it, which is why every input bumps the generation that the ref
 * name carries.
 */
export interface DeviceElement {
  /** Opaque handle for `device_interact`. Only actionable elements carry one. */
  ref?: string
  role: string
  label?: string
  value?: string
  /**
   * `accessibilityIdentifier` from the app's own source — the greppable link
   * back to the code, and unlike a web dev-build debug id it survives release.
   * A convention rather than a guarantee, so never depend on its presence.
   */
  uniqueId?: string
  disabled?: boolean
  /** In points. */
  frame?: { x: number; y: number; width: number; height: number }
}

export interface DeviceScreenRead {
  udid: string
  elements: DeviceElement[]
  /** Pass back as `cursor` to continue. Absent when the screen is exhausted. */
  nextCursor?: string
  /** Bumped after every input; refs from an older generation are refused. */
  generation: number
  /** Screen size in points — the space every ref and frame is expressed in. */
  screen: { width: number; height: number }
  /**
   * Pixels ÷ points, typically 3. Ships alongside every screenshot so a
   * coordinate read off an image can be converted rather than guessed: tapping
   * an image pixel directly lands at a third of the intended position.
   */
  scale: number
}

/** Where an interaction lands: a ref from `read_screen`, or a point (points). */
export type DeviceTarget = { ref: string } | { x: number; y: number }

/**
 * What the person pointed at on the device screen.
 *
 * The mobile counterpart of `BrowserSelection`, and thinner by necessity: there
 * is no markup, no selector and no component name behind a simulator, so the
 * accessibility node is the whole of what can be said about an element. That
 * makes `uniqueId` — the app's own `accessibilityIdentifier` — by far the most
 * valuable field here, since it is the one string that greps back to source.
 */
export interface DeviceSelection {
  udid: string
  /** The element under the point, deepest match. Absent if the point hit
   *  nothing describable, which is ordinary on blank areas. */
  element?: DeviceElement
  /** Where the person pointed, in points. */
  point: DevicePoint
  /** The screen this was read against; refs are void once it moves on. */
  generation: number
}

/**
 * Freehand ink over the device screen, resolved to what it covers.
 *
 * No image comes back, unlike the browser's annotation: the pane already holds
 * the frame the person drew on, and shipping a second copy of a multi-megabyte
 * screenshot through IPC to say the same thing is not worth it. The elements
 * are the part the renderer cannot work out for itself.
 */
export interface DeviceAnnotation {
  udid: string
  /** Elements under the ink, nearest-first, with refs where actionable. */
  elements: DeviceElement[]
  /** The ink's bounding box, in points. */
  bounds: { x: number; y: number; width: number; height: number }
  generation: number
}
