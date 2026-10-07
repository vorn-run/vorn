import path from 'node:path'
import os from 'node:os'
import fs from 'node:fs'
import log from './logger'
import { getDefaultShell } from './process-utils'
import { activeCore, nativeCore, type NativeStore, type NativeStoreClass } from './native-core'
import type {
  ArtifactCommentChange,
  ArtifactCommentFilter,
  ArtifactFilter,
  ConnectorInboxClaim,
  ConnectorInboxItem,
  ConnectorInboxRetry,
  ConnectorPollError,
  ConnectorPollPage,
  DeviceTokenSecret,
  NewArtifact,
  NewArtifactComment,
  NewDeviceToken,
  WebhookEvent
} from './store-types'

export type {
  ConnectorInboxItem,
  DeviceTokenSecret,
  NewArtifact,
  NewDeviceToken
} from './store-types'
import type {
  Artifact,
  ArtifactAuthor,
  ArtifactComment,
  ArtifactVersion
} from '@vornrun/shared/types'
import {
  AppConfig,
  ProjectConfig,
  WorkflowDefinition,
  WorkflowExecution,
  SSHKey,
  SSHKeyMeta,
  TaskConfig,
  TerminalSession,
  ScheduleLogEntry,
  WorkspaceConfig,
  SessionGroupConfig,
  DEFAULT_WORKSPACE,
  SessionEvent,
  SessionEventType,
  SourceConnection,
  TaskSourceLink,
  User,
  DeviceToken
} from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { buildDefaultTaskWorkflow, buildDevServerWorkflow } from './default-workflows'

/** Where the data directory lands when nothing overrides it. */
const DEFAULT_DATA_DIR = path.join(os.homedir(), '.vorn')

// Resolved by initDatabase() rather than fixed at module load, so a standalone
// server can be pointed at its own directory with --data-dir. The desktop
// passes nothing and keeps ~/.vorn — see the note in server-launcher.ts for why
// it must not pass Electron's userData.
//
// Only the directory is stored; every path under it is derived. Keeping a
// second `dbPath` variable in step by hand is how one code path ends up opening
// one directory's database while writing another's .db-signal.
let resolvedDataDir: string | null = null

function dbPath(): string {
  return path.join(getDataDir(), 'vorn.db')
}

/** The open database: the Rust store (`vorn-store`) in `vorn_core.node`. */
let store: NativeStore | null = null

/**
 * One call into the Rust store: the function's name and its arguments, as
 * JSON both ways. `undefined` crosses as null, which every call treats as
 * absent.
 */
function nativeCall<T>(call: string, ...args: unknown[]): T {
  if (!store) throw new Error('Database not initialized. Call initDatabase() first.')
  return JSON.parse(store.call(call, JSON.stringify(args))) as T
}

/**
 * The resolved data directory — the one place anything in this process should
 * ask where Vorn's files live.
 *
 * Throws rather than falling back to the default, because a caller that runs
 * before `initDatabase()` would otherwise get a plausible-looking `~/.vorn` and
 * quietly read or watch the wrong directory forever.
 */
export function getDataDir(): string {
  if (!resolvedDataDir) {
    throw new Error('Data directory not resolved. Call initDatabase() first.')
  }
  return resolvedDataDir
}

/**
 * Opens, migrates and seeds the database. A corrupt file is copied aside and
 * replaced by a fresh one, and where the copy went is logged.
 */
export function initDatabase(dataDir?: string): void {
  resolvedDataDir = dataDir ?? DEFAULT_DATA_DIR

  if (!fs.existsSync(getDataDir())) {
    fs.mkdirSync(getDataDir(), { recursive: true, mode: 0o700 })
  }

  closeDatabase()
  store = openStore((Store) => Store.open(dbPath(), nativeOptions()))
  if (store.recovered) {
    log.warn(
      `[database] Database was corrupted and has been reset. Backup saved to: ${store.recovered}`
    )
  }
}

function openStore(open: (Store: NativeStoreClass) => NativeStore): NativeStore {
  const Store = nativeCore()?.NativeStore
  if (typeof Store !== 'function') {
    throw new Error(
      `The database cannot open: the native core is not loaded, or was built without the store (${
        activeCore().error ?? 'no error reported'
      })`
    )
  }
  return open(Store)
}

/** What the Rust store needs that only the server knows. */
function nativeOptions(): string {
  let ownerName = 'owner'
  try {
    ownerName = os.userInfo().username || ownerName
  } catch {
    // No OS user available (some sandboxes) — the fallback is fine.
  }
  return JSON.stringify({
    defaultShell: getDefaultShell(),
    defaultAgentCommands: DEFAULT_AGENT_COMMANDS,
    defaultWorkspace: DEFAULT_WORKSPACE,
    ownerName,
    seedWorkflows: [
      { flag: 'hasSeededDefaultTaskWorkflow', workflow: buildDefaultTaskWorkflow() },
      { flag: 'hasSeededDevServerWorkflow', workflow: buildDevServerWorkflow() }
    ]
  })
}

/**
 * Insert the seeded "Default Task Workflow" on first launch. Gated by the
 * `hasSeededDefaultTaskWorkflow` defaults flag: once the user has seen (and
 * possibly deleted) the seeded workflow, we never re-seed. This means delete
 * sticks, and users upgrading from a pre-seed version will get it once.
 *
 * Exported so tests can exercise the seeding flow against an in-memory
 * database via `initTestDatabase` without spinning up the full init path.
 */
export function seedSystemDefaults(): void {
  return nativeCall('seedSystemDefaults')
}

/**
 * Touch a signal file so the config-manager watcher detects external DB mutations
 * (e.g. from MCP stdio process). The server's own mutations use notifyChanged() directly.
 */
export function dbSignalChange(): void {
  try {
    const signalPath = path.join(getDataDir(), '.db-signal')
    fs.writeFileSync(signalPath, Date.now().toString())
  } catch {
    // Best-effort — the next signal or local change reloads it
  }
}

export function closeDatabase(): void {
  store?.close()
  store = null
}

/** Initialize an in-memory database for tests. Returns teardown function. */
export function initTestDatabase(): () => void {
  closeDatabase()
  // The database is in memory, but anything deriving a path from the data dir
  // (dbSignalChange, task images) still needs one resolved. Point it at the temp
  // directory so tests cannot write into the developer's real ~/.vorn.
  resolvedDataDir = os.tmpdir()
  store = openStore((Store) => Store.openInMemory(nativeOptions()))
  return () => closeDatabase()
}

// ---------------------------------------------------------------------------
// Config: load
// ---------------------------------------------------------------------------

export function loadConfig(): AppConfig {
  return nativeCall('loadConfig')
}

/**
 * `saveConfig`'s arguments for the Rust store. JSON drops `undefined`, which
 * means something here: a default set to it is deleted, and an agent command
 * set to it keeps its row. The keys of the first cross on their own, and the
 * second crosses as null.
 */
function nativeConfig(config: AppConfig): [AppConfig, string[]] {
  const deleted = Object.entries(config.defaults)
    .filter(([, value]) => value === undefined)
    .map(([key]) => key)
  const agentCommands =
    config.agentCommands &&
    Object.fromEntries(Object.entries(config.agentCommands).map(([key, cmd]) => [key, cmd ?? null]))
  return [{ ...config, ...(agentCommands && { agentCommands }) }, deleted]
}

export function saveConfig(config: AppConfig): void {
  return nativeCall('saveConfig', ...nativeConfig(config))
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Tasks
// ---------------------------------------------------------------------------

export function dbListTasks(projectName?: string, status?: string): TaskConfig[] {
  return nativeCall('dbListTasks', projectName, status)
}

export function dbGetTask(id: string): TaskConfig | null {
  return nativeCall('dbGetTask', id)
}

export function dbInsertTask(task: TaskConfig): void {
  return nativeCall('dbInsertTask', task)
}

export function dbUpdateTask(id: string, updates: Partial<TaskConfig>): void {
  return nativeCall('dbUpdateTask', id, updates, Object.keys(updates))
}

export function dbDeleteTask(id: string): void {
  return nativeCall('dbDeleteTask', id)
}

export function dbGetMaxTaskOrder(projectName: string): number {
  return nativeCall('dbGetMaxTaskOrder', projectName)
}

export function dbListSourceConnections(connectorId?: string): SourceConnection[] {
  return nativeCall('dbListSourceConnections', connectorId)
}

export function dbGetSourceConnection(id: string): SourceConnection | null {
  return nativeCall('dbGetSourceConnection', id)
}

export function dbInsertSourceConnection(conn: SourceConnection): void {
  return nativeCall('dbInsertSourceConnection', conn)
}

export function dbUpdateSourceConnection(id: string, updates: Partial<SourceConnection>): void {
  return nativeCall('dbUpdateSourceConnection', id, updates, Object.keys(updates))
}

/** Who a connection's window is signed in as; both null once it is signed out. */
export function dbSetConnectionSignIn(
  id: string,
  signedInAs: string | null,
  signedInAt: string | null
): void {
  return nativeCall('dbSetConnectionSignIn', id, signedInAs, signedInAt)
}

export function dbDeleteSourceConnection(id: string): void {
  return nativeCall('dbDeleteSourceConnection', id)
}

export function dbGetConnectorPollCursor(
  workflowId: string,
  connectionId: string
): string | undefined {
  return (
    nativeCall<string | null>('dbGetConnectorPollCursor', workflowId, connectionId) ?? undefined
  )
}

export function dbCountActiveConnectorInboxLeases(now: string): number {
  return nativeCall('dbCountActiveConnectorInboxLeases', now)
}

/**
 * Persist one remote page and its checkpoint in a single transaction.
 * A crash can leave both absent or both present, never a cursor that points
 * beyond events which were only held in memory.
 */
/** One webhook request becomes one durable inbox row. */
export function dbEnqueueWebhookEvent(args: WebhookEvent): void {
  return nativeCall('dbEnqueueWebhookEvent', args)
}

export function dbRecordConnectorPollPage(args: ConnectorPollPage): number {
  return nativeCall('dbRecordConnectorPollPage', args)
}

export function dbRecordConnectorPollError(args: ConnectorPollError): void {
  return nativeCall('dbRecordConnectorPollError', args)
}

/**
 * Lease ready inbox rows before broadcasting them. Expired leases are
 * reclaimable after a renderer/server crash. Retry backoff caps at one hour,
 * but rows remain pending until they succeed or the user removes their source.
 */
export function dbClaimConnectorInbox(args: ConnectorInboxClaim): ConnectorInboxItem[] {
  return nativeCall('dbClaimConnectorInbox', args)
}

export function dbCompleteConnectorInbox(
  id: number,
  leaseToken: string,
  processedAt: string
): boolean {
  return nativeCall('dbCompleteConnectorInbox', id, leaseToken, processedAt)
}

/** Retries stop here: a row this old is failing for a reason a retry won't fix. */
export const MAX_INBOX_ATTEMPTS = 8

export function dbRetryConnectorInbox(args: ConnectorInboxRetry): boolean {
  return nativeCall('dbRetryConnectorInbox', args)
}

/** The renderer could not accept this event yet (for example, another run is
 * parked at an approval gate). This is not a workflow attempt or an error. */
export function dbDeferConnectorInbox(
  id: number,
  leaseToken: string,
  availableAt: string
): boolean {
  return nativeCall('dbDeferConnectorInbox', id, leaseToken, availableAt)
}

export function dbRenewConnectorInboxLease(
  id: number,
  leaseToken: string,
  leaseUntil: string
): boolean {
  return nativeCall('dbRenewConnectorInboxLease', id, leaseToken, leaseUntil)
}

/** Server restarts invalidate every in-memory workflow owner, so leases from
 * the previous process must be immediately reclaimable. */
export function dbReleaseConnectorInboxLeases(now: string): void {
  return nativeCall('dbReleaseConnectorInboxLeases', now)
}

export function dbGetTaskSourceLink(taskId: string): TaskSourceLink | null {
  return nativeCall('dbGetTaskSourceLink', taskId)
}

export function dbGetTaskSourceLinkByExternalId(
  connectionId: string,
  externalId: string
): TaskSourceLink | null {
  return nativeCall('dbGetTaskSourceLinkByExternalId', connectionId, externalId)
}

/**
 * Fallback lookup for orphan re-linking: find a task whose own
 * sourceConnectorId/sourceExternalId matches, even if its task_source_links
 * row is missing (e.g. because a prior connection was deleted and cascaded
 * the link). Used by the import path to re-adopt existing tasks instead of
 * creating duplicates.
 */
export function dbFindTaskByConnectorExternalId(
  connectorId: string,
  externalId: string
): TaskConfig | null {
  return nativeCall('dbFindTaskByConnectorExternalId', connectorId, externalId)
}

export function dbListTaskSourceLinks(connectionId: string): TaskSourceLink[] {
  return nativeCall('dbListTaskSourceLinks', connectionId)
}

export function dbInsertTaskSourceLink(link: TaskSourceLink): void {
  return nativeCall('dbInsertTaskSourceLink', link)
}

export function dbUpdateTaskSourceLink(taskId: string, updates: Partial<TaskSourceLink>): void {
  return nativeCall('dbUpdateTaskSourceLink', taskId, updates)
}

export function dbDeleteTaskSourceLink(taskId: string): void {
  return nativeCall('dbDeleteTaskSourceLink', taskId)
}

export function dbListProjects(): ProjectConfig[] {
  return nativeCall('dbListProjects')
}

export function dbGetProject(name: string): ProjectConfig | null {
  return nativeCall('dbGetProject', name)
}

export function dbInsertProject(project: ProjectConfig): void {
  return nativeCall('dbInsertProject', project)
}

export function dbUpdateProject(name: string, updates: Partial<ProjectConfig>): void {
  return nativeCall('dbUpdateProject', name, updates)
}

export function dbDeleteProject(name: string): void {
  return nativeCall('dbDeleteProject', name)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Workflows
// ---------------------------------------------------------------------------

export function dbListWorkflows(): WorkflowDefinition[] {
  return nativeCall('dbListWorkflows')
}

export function dbGetWorkflow(id: string): WorkflowDefinition | null {
  return nativeCall('dbGetWorkflow', id)
}

export function dbInsertWorkflow(workflow: WorkflowDefinition): void {
  return nativeCall('dbInsertWorkflow', workflow)
}

/**
 * Change a workflow's columns, and say how many rows changed.
 *
 * Zero has two causes, and a caller must know which it is asking about: no row
 * matched the id, or `updates` named nothing this function writes. It is an
 * existence answer only for a caller that passed at least one writable column —
 * `workflow:setEnabled` always passes `enabled`, so for that one it is.
 *
 * The count is worth having because the alternative is worse. Reading the row
 * first with `dbGetWorkflow` costs a second statement and a `JSON.parse` of
 * `nodes` and `edges`, so one malformed workflow could throw a caller that only
 * wanted to flip a boolean — and it leaves a gap between the check and the write
 * for the row to disappear in.
 */
export function dbUpdateWorkflow(id: string, updates: Partial<WorkflowDefinition>): number {
  return nativeCall('dbUpdateWorkflow', id, updates)
}

export function dbDeleteWorkflow(id: string): void {
  return nativeCall('dbDeleteWorkflow', id)
}

/** The seeded owner. Present after migration 14 on any initialized database. */
export function dbGetOwnerUser(): User | null {
  return nativeCall('dbGetOwnerUser')
}

export function dbInsertDeviceToken(token: NewDeviceToken): void {
  return nativeCall('dbInsertDeviceToken', token)
}

/** Carries the hash — for verification only. */
export function dbGetDeviceTokenSecret(id: string): DeviceTokenSecret | null {
  return nativeCall('dbGetDeviceTokenSecret', id)
}

/**
 * Columns are named rather than `SELECT *`, following `dbListSSHKeys`, so the
 * hash never leaves the data layer even if a later `...row` spread is careless.
 */
export function dbListDeviceTokens(): DeviceToken[] {
  return nativeCall('dbListDeviceTokens')
}

/** Cheaper than listing when the caller only wants to know whether any exist. */
export function dbHasDeviceTokens(): boolean {
  return nativeCall('dbHasDeviceTokens')
}

/** Returns false when the id is unknown or the token was already revoked. */
export function dbRevokeDeviceToken(id: string, revokedAt: string): boolean {
  return nativeCall('dbRevokeDeviceToken', id, revokedAt)
}

export function dbTouchDeviceToken(id: string, seenAt: string): void {
  return nativeCall('dbTouchDeviceToken', id, seenAt)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Workspaces
// ---------------------------------------------------------------------------

export function dbListWorkspaces(): WorkspaceConfig[] {
  return nativeCall('dbListWorkspaces')
}

export function dbInsertWorkspace(workspace: WorkspaceConfig): void {
  return nativeCall('dbInsertWorkspace', workspace)
}

export function dbUpdateWorkspace(id: string, updates: Partial<WorkspaceConfig>): void {
  return nativeCall('dbUpdateWorkspace', id, updates)
}

export function dbDeleteWorkspace(id: string): void {
  return nativeCall('dbDeleteWorkspace', id)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Session groups
// ---------------------------------------------------------------------------

export function dbListSessionGroups(): SessionGroupConfig[] {
  return nativeCall('dbListSessionGroups')
}

export function dbInsertSessionGroup(group: SessionGroupConfig): void {
  return nativeCall('dbInsertSessionGroup', group)
}

export function dbUpdateSessionGroup(id: string, updates: Partial<SessionGroupConfig>): void {
  return nativeCall('dbUpdateSessionGroup', id, updates)
}

export function dbDeleteSessionGroup(id: string): void {
  return nativeCall('dbDeleteSessionGroup', id)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: SSH Keys
// ---------------------------------------------------------------------------

export function dbSaveSSHKey(key: SSHKey): void {
  return nativeCall('dbSaveSSHKey', key)
}

export function dbListSSHKeys(): SSHKeyMeta[] {
  return nativeCall('dbListSSHKeys')
}

export function dbGetSSHKey(id: string): SSHKey | null {
  return nativeCall('dbGetSSHKey', id)
}

export function dbDeleteSSHKey(id: string): void {
  return nativeCall('dbDeleteSSHKey', id)
}

// ---------------------------------------------------------------------------
// Granular updates (avoids full load/save cycle for hot paths)
// ---------------------------------------------------------------------------

export function updateWorkflowRunStatus(
  id: string,
  lastRunAt: string,
  lastRunStatus: string
): void {
  return nativeCall('updateWorkflowRunStatus', id, lastRunAt, lastRunStatus)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export function saveSessions(sessions: TerminalSession[]): void {
  return nativeCall('saveSessions', sessions)
}

export function getPreviousSessions(): TerminalSession[] {
  return nativeCall('getPreviousSessions')
}

export function clearSessions(): void {
  return nativeCall('clearSessions')
}

// ---------------------------------------------------------------------------
// Schedule log
// ---------------------------------------------------------------------------

export function addScheduleLogEntry(entry: ScheduleLogEntry): void {
  return nativeCall('addScheduleLogEntry', entry)
}

export function getScheduleLogEntries(workflowId?: string): ScheduleLogEntry[] {
  return nativeCall('getScheduleLogEntries', workflowId)
}

export function clearScheduleLog(): void {
  return nativeCall('clearScheduleLog')
}

/** Every run id kept, so review pages of runs trimmed while the server was down can go too. */
export function listWorkflowRunIds(): string[] {
  return nativeCall('listWorkflowRunIds')
}

/** A run as clients get it: the definition snapshot is the engine's, and heavy on every update. */
export function withoutDefinition<T extends WorkflowExecution>(run: T): Omit<T, 'definition'> {
  const { definition: _definition, ...rest } = run
  return rest
}

export function dbGetWorkflowRunByConnectorInboxId(
  connectorInboxId: number
): WorkflowExecution | null {
  return nativeCall('dbGetWorkflowRunByConnectorInboxId', connectorInboxId)
}

/** One run by its id — what the engine reads when a gate is answered after a restart. */
export function getWorkflowRun(runId: string): WorkflowExecution | null {
  return nativeCall('getWorkflowRun', runId)
}

export function listWorkflowRuns(workflowId: string, limit = 20): WorkflowExecution[] {
  return nativeCall('listWorkflowRuns', workflowId, limit)
}

export function listWorkflowRunsByTask(
  taskId: string,
  limit = 20
): (WorkflowExecution & { workflowName?: string })[] {
  return nativeCall('listWorkflowRunsByTask', taskId, limit)
}

/**
 * Runs in `running` state. Used at renderer startup to reconcile orphaned
 * runs: when the renderer reloads mid-execution, headless agents in the main
 * process keep going but the in-memory exit-promise dies, leaving the run
 * stuck. The reconciler closes these out against `session_events`.
 */
export function listRunningRuns(): WorkflowExecution[] {
  return nativeCall('listRunningRuns')
}

// Surfaces every run that has at least one waiting node — small in practice
// because gates pause execution. No LIMIT is intentional so the badge count
// matches the real backlog. If this ever grows, cap with a LIMIT here and
// chunk `fetchNodesByRunIds` to stay under SQLite's IN-clause variable cap.
export function listRunsWithWaitingGates(kind?: 'signIn'): WorkflowExecution[] {
  return nativeCall('listRunsWithWaitingGates', kind)
}

/**
 * Cross-workflow run history for the Workflows → All runs view. Joins on
 * the workflows table so the renderer can display the workflow name without
 * a second lookup. When `workspaceId` is provided, restricts to workflows
 * in that workspace; otherwise returns runs across every workflow.
 */
export function listAllWorkflowRuns(
  workspaceId?: string,
  limit = 50
): (WorkflowExecution & { workflowName?: string })[] {
  return nativeCall('listAllWorkflowRuns', workspaceId, limit)
}

/**
 * Records that the effect `effectId` was acted on. True the first time, false
 * for every delivery after: the caller acts only on true.
 */
export function claimEffect(effectId: string, kind: string, now = Date.now()): boolean {
  return nativeCall('claimEffect', effectId, kind, now)
}

/** Forgets receipts of `kind` older than `before`, and answers how many went. */
export function pruneEffectReceipts(kind: string, before: number): number {
  return nativeCall('pruneEffectReceipts', kind, before)
}

export function insertSessionEvent(event: SessionEvent): void {
  return nativeCall('insertSessionEvent', event)
}

export function listSessionEvents(eventType?: SessionEventType, limit = 100): SessionEvent[] {
  return nativeCall('listSessionEvents', eventType, limit)
}

export function listSessionEventsBySession(sessionId: string, limit = 100): SessionEvent[] {
  return nativeCall('listSessionEventsBySession', sessionId, limit)
}

// ─── Artifacts ────────────────────────────────────────────────────

/** Create an artifact with no versions yet, and the token that unlocks its pages. */
export function insertArtifact(fields: NewArtifact): { artifact: Artifact; token: string } {
  return nativeCall('insertArtifact', fields)
}

export function getArtifact(id: string): Artifact | null {
  return nativeCall('getArtifact', id)
}

export function getArtifactToken(id: string): string | null {
  return nativeCall('getArtifactToken', id)
}

/** The newest first, narrowed to one session or one project when asked. */
export function listArtifacts(filter: ArtifactFilter = {}, limit = 50): Artifact[] {
  return nativeCall('listArtifacts', filter, limit)
}

/** The artifact a gate's review pages are kept as, one version per round. */
export function findGateArtifact(runId: string, nodeId: string): Artifact | null {
  return nativeCall('findGateArtifact', runId, nodeId)
}

export function renameArtifact(id: string, title: string): void {
  return nativeCall('renameArtifact', id, title)
}

/** Number the next version and make it the latest, in one step so two publishes cannot share a number. */
export function addArtifactVersion(
  artifactId: string,
  author: ArtifactAuthor,
  answersBatchId?: string
): ArtifactVersion {
  return nativeCall('addArtifactVersion', artifactId, author, answersBatchId)
}

export function listArtifactVersions(artifactId: string): ArtifactVersion[] {
  return nativeCall('listArtifactVersions', artifactId)
}

/** The latest batch sent on this artifact that no version has answered yet. */
export function unansweredBatchId(artifactId: string): string | undefined {
  return nativeCall<string | null>('unansweredBatchId', artifactId) ?? undefined
}

export function listArtifactComments(
  artifactId: string,
  filter: ArtifactCommentFilter = {}
): ArtifactComment[] {
  return nativeCall('listArtifactComments', artifactId, filter)
}

export function getArtifactComment(id: string): ArtifactComment | null {
  return nativeCall('getArtifactComment', id)
}

export function insertArtifactComment(fields: NewArtifactComment): ArtifactComment {
  return nativeCall('insertArtifactComment', fields)
}

/** Change a draft's words or anchor; a sent comment is part of the record and stays as it was. */
export function updateArtifactComment(
  id: string,
  change: ArtifactCommentChange
): ArtifactComment | null {
  return nativeCall('updateArtifactComment', id, change)
}

/** Drop a draft. Returns false when there was no draft by that id. */
export function deleteArtifactComment(id: string): boolean {
  return nativeCall('deleteArtifactComment', id)
}

/** Seal every draft on the artifact into one batch. Null when there were none. */
export function sendArtifactDrafts(
  artifactId: string
): { batchId: string; comments: ArtifactComment[] } | null {
  return nativeCall('sendArtifactDrafts', artifactId)
}

/** Remove artifacts untouched since `cutoff`, returning their ids so their pages can go too. */
export function deleteArtifactsUpdatedBefore(cutoff: string): string[] {
  return nativeCall('deleteArtifactsUpdatedBefore', cutoff)
}

export function listArtifactIds(): string[] {
  return nativeCall('listArtifactIds')
}
