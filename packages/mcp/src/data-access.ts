import type {
  AppConfig,
  ProjectConfig,
  TaskConfig,
  WorkflowDefinition,
  WorkspaceConfig,
  WorkflowExecution
} from '@vornrun/shared/types'
import { rpcCall } from './ws-client'

/**
 * Tasks, projects, workspaces and workflows, read and written over the socket.
 *
 * These four used to open `~/.vorn/vorn.db` directly. That was fine while the only
 * server was this machine's, and wrong the moment the desktop could be pointed at
 * a host: the data is over there, and the local file is a stale copy nobody is
 * serving. Answering from it is worse than failing, because an agent reads the
 * wrong backlog and reports confidently on it.
 *
 * Everything here lives in the config blob, which `config:load` and `config:save`
 * already carry — so this needs no new server methods, and it goes to whichever
 * server this MCP is talking to, local or remote, without knowing which.
 *
 * Read-modify-write per call is safe now that a save carries the revision it was
 * based on: the server keeps rows added by anyone else since, rather than pruning
 * them as deletions. Before that this pattern would have been the very thing that
 * lost data.
 */
async function loadConfig(): Promise<AppConfig> {
  return rpcCall<AppConfig>('config:load')
}

async function saveConfig(config: AppConfig): Promise<void> {
  await rpcCall('config:save', config)
}

/** Load, change, save. The shape every mutation below takes. */
async function mutate(change: (config: AppConfig) => AppConfig): Promise<void> {
  const config = await loadConfig()
  await saveConfig(change(config))
}

// ─── Projects ────────────────────────────────────────────────────

export async function dbListProjects(): Promise<ProjectConfig[]> {
  return (await loadConfig()).projects ?? []
}

export async function dbGetProject(name: string): Promise<ProjectConfig | null> {
  return (await dbListProjects()).find((p) => p.name === name) ?? null
}

export async function dbInsertProject(project: ProjectConfig): Promise<void> {
  await mutate((config) => ({ ...config, projects: [...(config.projects ?? []), project] }))
}

export async function dbUpdateProject(
  name: string,
  updates: Partial<ProjectConfig>
): Promise<void> {
  await mutate((config) => ({
    ...config,
    projects: (config.projects ?? []).map((p) => (p.name === name ? { ...p, ...updates } : p))
  }))
}

export async function dbDeleteProject(name: string): Promise<void> {
  await mutate((config) => ({
    ...config,
    projects: (config.projects ?? []).filter((p) => p.name !== name)
  }))
}

// ─── Workspaces ──────────────────────────────────────────────────

export async function dbListWorkspaces(): Promise<WorkspaceConfig[]> {
  return (await loadConfig()).workspaces ?? []
}

export async function dbInsertWorkspace(workspace: WorkspaceConfig): Promise<void> {
  await mutate((config) => ({ ...config, workspaces: [...(config.workspaces ?? []), workspace] }))
}

export async function dbUpdateWorkspace(
  id: string,
  updates: Partial<WorkspaceConfig>
): Promise<void> {
  await mutate((config) => ({
    ...config,
    workspaces: (config.workspaces ?? []).map((w) => (w.id === id ? { ...w, ...updates } : w))
  }))
}

export async function dbDeleteWorkspace(id: string): Promise<void> {
  await mutate((config) => ({
    ...config,
    workspaces: (config.workspaces ?? []).filter((w) => w.id !== id)
  }))
}

// ─── Workflows ───────────────────────────────────────────────────

export async function dbListWorkflows(): Promise<WorkflowDefinition[]> {
  return (await loadConfig()).workflows ?? []
}

export async function dbInsertWorkflow(workflow: WorkflowDefinition): Promise<void> {
  await mutate((config) => ({ ...config, workflows: [...(config.workflows ?? []), workflow] }))
}

export async function dbUpdateWorkflow(
  id: string,
  updates: Partial<WorkflowDefinition>
): Promise<void> {
  await mutate((config) => ({
    ...config,
    workflows: (config.workflows ?? []).map((w) => (w.id === id ? { ...w, ...updates } : w))
  }))
}

export async function dbDeleteWorkflow(id: string): Promise<void> {
  await mutate((config) => ({
    ...config,
    workflows: (config.workflows ?? []).filter((w) => w.id !== id)
  }))
}

// ─── Tasks ───────────────────────────────────────────────────────

export async function dbListTasks(projectName?: string, status?: string): Promise<TaskConfig[]> {
  const tasks = (await loadConfig()).tasks ?? []
  return tasks.filter(
    (t) =>
      (projectName === undefined || t.projectName === projectName) &&
      (status === undefined || t.status === status)
  )
}

export async function dbGetTask(id: string): Promise<TaskConfig | null> {
  return ((await loadConfig()).tasks ?? []).find((t) => t.id === id) ?? null
}

export async function dbInsertTask(task: TaskConfig): Promise<void> {
  await mutate((config) => ({ ...config, tasks: [...(config.tasks ?? []), task] }))
}

export async function dbUpdateTask(id: string, updates: Partial<TaskConfig>): Promise<void> {
  await mutate((config) => ({
    ...config,
    tasks: (config.tasks ?? []).map((t) => (t.id === id ? { ...t, ...updates } : t))
  }))
}

export async function dbDeleteTask(id: string): Promise<void> {
  await mutate((config) => ({
    ...config,
    tasks: (config.tasks ?? []).filter((t) => t.id !== id)
  }))
}

export async function dbGetMaxTaskOrder(projectName: string): Promise<number> {
  const tasks = await dbListTasks(projectName)
  return tasks.reduce((max, t) => Math.max(max, t.order ?? 0), 0)
}

/**
 * A no-op, kept so the call sites read the same.
 *
 * It existed to poke the database file so the server would notice an outside
 * writer and re-broadcast. Writing through `config:save` means the server is the
 * writer, and it already broadcasts — there is nothing left to signal.
 */
export function dbSignalChange(): void {}

// ─── Workflow runs ───────────────────────────────────────────────

/**
 * Runs live in their own tables rather than the config blob, so these go through
 * the RPCs that already serve them rather than the load/save pair above.
 */
export async function listWorkflowRuns(
  workflowId: string,
  limit = 20
): Promise<WorkflowExecution[]> {
  return rpcCall<WorkflowExecution[]>('workflowRun:list', { workflowId, limit })
}

export async function listWorkflowRunsByTask(
  taskId: string,
  limit = 20
): Promise<(WorkflowExecution & { workflowName?: string })[]> {
  return rpcCall<(WorkflowExecution & { workflowName?: string })[]>('workflowRun:listByTask', {
    taskId,
    limit
  })
}

export async function listAllWorkflowRuns(
  workspaceId?: string,
  limit = 50
): Promise<(WorkflowExecution & { workflowName?: string })[]> {
  return rpcCall<(WorkflowExecution & { workflowName?: string })[]>('workflowRun:listAll', {
    workspaceId,
    limit
  })
}
