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
  /** Shell session only: working directory the PTY was started in. */
  shellCwd?: string
  /** Shell session only: PTY exit code once the shell has exited. */
  shellExitCode?: number
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

export interface PollResult {
  events: TriggerEvent[]
  nextCursor?: string
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
  }
  connectorItem?: ConnectorItemContext
}

export type WorkflowNodeType =
  | 'trigger'
  | 'launchAgent'
  | 'script'
  | 'condition'
  | 'approval'
  | 'createTaskFromItem'
  | 'callConnectorAction'

export interface WorkflowNodePosition {
  x: number
  y: number
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
export type TriggerConfig =
  | ManualTriggerConfig
  | OnceTriggerConfig
  | RecurringTriggerConfig
  | TaskCreatedTriggerConfig
  | TaskStatusChangedTriggerConfig
  | ConnectorPollTriggerConfig

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
  /** Raw args map; values support template placeholders. */
  args: Record<string, string>
}

export type WorkflowNodeConfig =
  | TriggerConfig
  | LaunchAgentConfig
  | ScriptConfig
  | ConditionConfig
  | ApprovalConfig
  | CreateTaskFromItemConfig
  | CallConnectorActionConfig

export interface WorkflowNode {
  id: string
  type: WorkflowNodeType
  label: string
  slug?: string
  config: WorkflowNodeConfig
  position: WorkflowNodePosition
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
  startedAt?: string
  completedAt?: string
  sessionId?: string
  error?: string
  logs?: string
  output?: string
  /**
   * Typed payload returned by a callConnectorAction step (the connector's
   * `ActionResult.output`, which matches the action's declared
   * `outputSchema`). Stored separately from the string `output` / `logs`
   * fields so the template resolver can walk nested paths like
   * `{{steps.create_issue.html_url}}` at its original shape.
   */
  structuredOutput?: Record<string, unknown>
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
  workflowId: string
  startedAt: string
  completedAt?: string
  status: 'running' | 'success' | 'error'
  nodeStates: NodeExecutionState[]
  triggerTaskId?: string
}

// ─── Tailscale Network Access ────────────────────────────────────

export interface TailscalePeer {
  ip: string
  hostname: string
  dnsName: string
  os: string
  online: boolean
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

export interface AppConfig {
  version: number
  defaults: {
    shell: string
    fontSize: number
    theme: 'dark' | 'light'
    rowHeight?: number
    defaultAgent?: AiAgentType
    notifications?: NotificationConfig
    hasSeenOnboarding?: boolean | number
    reopenSessions?: boolean
    widgetEnabled?: boolean
    taskViewMode?: TaskViewMode
    layoutMode?: 'grid' | 'tabs'
    minimizedPlacement?: MinimizedPlacement
    mainViewMode?: MainViewMode
    activeWorkspace?: string
    updateChannel?: 'stable' | 'beta'
    webAccessEnabled?: boolean
    mobileAccessEnabled?: boolean
    networkAccessEnabled?: boolean
    showHeadlessAgents?: boolean
    headlessRetentionMinutes?: number
    enableHoverPreview?: boolean
    /**
     * Set to `true` after the seeded "Default Task Workflow" has been inserted
     * once. Ensures deleting the workflow sticks — we don't resurrect it on
     * the next launch.
     */
    hasSeededDefaultTaskWorkflow?: boolean
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

export interface GitCommitPayload {
  cwd: string
  message: string
  includeUnstaged: boolean
}

export interface GitCommitResult {
  success: boolean
  error?: string
}

export const IPC = {
  TERMINAL_CREATE: 'terminal:create',
  TERMINAL_WRITE: 'terminal:write',
  TERMINAL_RESIZE: 'terminal:resize',
  TERMINAL_KILL: 'terminal:kill',
  TERMINAL_DATA: 'terminal:data',
  TERMINAL_EXIT: 'terminal:exit',
  SESSION_CREATED: 'session:created',
  SESSION_UPDATED: 'session:updated',
  SESSION_REORDERED: 'session:reordered',
  TERMINAL_RENAME: 'terminal:rename-session',
  TERMINAL_REORDER: 'terminal:reorder-sessions',
  CONFIG_LOAD: 'config:load',
  CONFIG_SAVE: 'config:save',
  CONFIG_CHANGED: 'config:changed',
  SESSIONS_GET_PREVIOUS: 'sessions:getPrevious',
  SESSIONS_CLEAR: 'sessions:clear',
  SESSIONS_GET_RECENT: 'sessions:getRecent',
  DIALOG_OPEN_DIRECTORY: 'dialog:openDirectory',
  IDE_DETECT: 'ide:detect',
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
  GIT_GET_BRANCH: 'git:getBranch',
  GIT_DIFF_STAT: 'git:diffStat',
  GIT_DIFF_FULL: 'git:diffFull',
  GIT_COMMIT: 'git:commit',
  GIT_PUSH: 'git:push',
  DIALOG_OPEN_FILE: 'dialog:openFile',
  SCHEDULER_EXECUTE: 'scheduler:execute',
  SCHEDULER_MISSED: 'scheduler:missed',
  SCHEDULER_GET_LOG: 'scheduler:getLog',
  SCHEDULER_GET_NEXT_RUN: 'scheduler:getNextRun',
  WORKFLOW_EXECUTION_COMPLETE: 'workflow:executionComplete',
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
  UPDATE_DOWNLOADED: 'update:downloaded',
  UPDATE_INSTALL: 'update:install',
  UPDATE_SET_CHANNEL: 'update:set-channel',
  TASK_IMAGE_SAVE: 'task:imageSave',
  TASK_IMAGE_DELETE: 'task:imageDelete',
  TASK_IMAGE_GET_PATH: 'task:imageGetPath',
  TASK_IMAGE_CLEANUP: 'task:imageCleanup',
  DIALOG_OPEN_IMAGE: 'dialog:openImage',
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
  SESSION_EVENT_LIST: 'sessionEvent:list',
  SESSION_EVENT_LIST_BY_SESSION: 'sessionEvent:listBySession',
  AGENT_DETECT_INSTALLED: 'agent:detectInstalled',
  TAILSCALE_STATUS: 'tailscale:status',
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
  FILE_WRITE_CONTENT: 'file:writeContent',
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
  CONNECTION_UPSERT_FROM_ITEM: 'connection:upsertFromItem',
  WORKFLOW_RUN_MANUAL: 'workflow:runManual',
  CONNECTION_BACKFILL: 'connection:backfill',
  CREDENTIALS_SET_DECRYPTED: 'credentials:setDecrypted',
  CREDENTIALS_CLEAR_DECRYPTED: 'credentials:clearDecrypted',
  CONNECTION_EXECUTE_ACTION: 'connection:executeAction',
  CONNECTION_LIST_ACTIONS: 'connection:listActions',
  CONNECTION_LIST_MCP_TOOLS: 'connection:listMcpTools',
  CONNECTION_REFRESH_MCP_TOOLS: 'connection:refreshMcpTools'
} as const

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
