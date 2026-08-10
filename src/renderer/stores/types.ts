import {
  AgentStatus,
  AiAgentType,
  AppConfig,
  ProjectConfig,
  WorkflowDefinition,
  WorkflowExecution,
  WorkspaceConfig,
  RemoteHost,
  TerminalSession,
  HeadlessSession,
  GitDiffStat,
  TaskConfig,
  TaskStatus
} from '../../shared/types'

export interface WorktreeInfo {
  path: string
  branch: string
  isMain: boolean
  name: string
  isDirty?: boolean
  diffStat?: { filesChanged: number; insertions: number; deletions: number }
  linkedSessionId?: string
}

export const MAIN_WORKTREE_SENTINEL = '__main__'

export type SortMode = 'manual' | 'created' | 'recent'
export type StatusFilter = AgentStatus | 'all'
export type TaskStatusFilter = 'all' | 'todo' | 'in_progress' | 'in_review' | 'done' | 'cancelled'

/** Filter tasks by source: 'all' shows everything, 'local' shows only local tasks,
 *  or a connector ID (e.g. 'github') shows only tasks from that connector.
 *  The `string & {}` branch preserves the literals for autocomplete while still
 *  accepting arbitrary connector ids. */
export type TaskSourceFilter = 'all' | 'local' | (string & {})
export type ProjectSortMode = 'manual' | 'name' | 'recent'
export type WorkflowFilter = 'all' | 'manual' | 'scheduled'
export type RunBucket = 'all' | 'running' | 'waiting' | 'success' | 'error'
export type WorktreeSortMode = 'name' | 'recent'
export type WorktreeFilter = 'all' | 'active'
export type SidebarViewMode = 'worktrees' | 'worktrees-sessions' | 'sessions' | 'sessions-flat'
export type PanelTab = 'changes' | 'all-files'

export interface FlexibleLayoutRect {
  x: number
  y: number
  w: number
  h: number
}

export interface TerminalState {
  id: string
  session: TerminalSession
  status: AgentStatus
  lastOutputTimestamp: number
}

export interface TerminalsSlice {
  terminals: Map<string, TerminalState>
  addTerminal: (session: TerminalSession) => void
  removeTerminal: (id: string) => void
  updateStatus: (id: string, status: AgentStatus) => void
  updateLastOutput: (id: string, timestamp: number) => void
  renameTerminal: (id: string, displayName: string) => void
  updateSessionBranch: (id: string, branch: string) => void
  updateSessionCwd: (id: string, shellCwd: string) => void
  setBranchForCwd: (cwd: string, branch: string) => void
  updateSessionWorktree: (
    id: string,
    updates: { worktreePath?: string; worktreeName?: string }
  ) => void

  // Headless agent tracking
  headlessSessions: HeadlessSession[]
  headlessLastOutput: Map<string, string>
  headlessDismissed: Set<string>
  setHeadlessSessions: (sessions: HeadlessSession[]) => void
  addHeadlessSession: (session: HeadlessSession) => void
  updateHeadlessSession: (id: string, updates: Partial<HeadlessSession>) => void
  dismissHeadlessSession: (id: string) => void
  pruneExitedHeadless: (retentionMs: number) => void
  setHeadlessLastOutput: (id: string, line: string) => void
}

export interface ProjectsSlice {
  config: AppConfig | null
  activeProject: string | null
  activeWorktreePath: string | null
  setConfig: (config: AppConfig) => void
  setActiveProject: (name: string | null) => void
  setActiveWorktreePath: (path: string | null) => void
  addProject: (project: ProjectConfig) => void
  removeProject: (name: string) => void
  updateProject: (originalName: string, project: ProjectConfig) => void
  addWorkflow: (workflow: WorkflowDefinition) => void
  removeWorkflow: (id: string) => void
  updateWorkflow: (id: string, workflow: WorkflowDefinition) => void
  addRemoteHost: (host: RemoteHost) => void
  removeRemoteHost: (id: string) => void
  updateRemoteHost: (id: string, host: RemoteHost) => void
  addWorkspace: (workspace: WorkspaceConfig) => void
  removeWorkspace: (id: string) => void
  updateWorkspace: (id: string, updates: Partial<WorkspaceConfig>) => void
}

export type SettingsCategory =
  | 'appearance'
  | 'general'
  | 'notifications'
  | 'agents'
  | 'worktrees'
  | 'ssh'
  | 'mcp'
  | 'connectors'
  | 'network'
  | 'about'

export interface UISlice {
  activeWorkspace: string
  focusedTerminalId: string | null
  selectedTerminalId: string | null
  previewTerminalId: string | null
  renamingTerminalId: string | null
  isSidebarOpen: boolean
  isNewAgentDialogOpen: boolean
  isAddProjectDialogOpen: boolean
  isWorkflowEditorOpen: boolean
  editingWorkflowId: string | null
  /**
   * A workflow whose manual run is waiting on the user. Set when the run was
   * triggered from a surface that can't supply everything the workflow needs —
   * a contextual workflow started from the sidebar / command palette (no
   * folder), or any workflow declaring run inputs. The SourcePromptDialog
   * renders while this is set. Cleared when the user submits or cancels.
   *
   * `context` is what the launching surface already knew (a card or terminal
   * right-click), letting the dialog prompt only for what's genuinely missing
   * instead of re-asking for a folder the caller already has. It lives inside
   * this object rather than beside it so a stale context can't outlive the
   * run it belongs to.
   */
  pendingWorkflowRun: {
    workflowId: string
    context?: { task?: TaskConfig; source?: TerminalSession }
  } | null
  editingProject: ProjectConfig | null
  isCommandPaletteOpen: boolean
  isShortcutsPanelOpen: boolean
  isSettingsOpen: boolean
  settingsCategory: SettingsCategory
  showSessionBanner: boolean
  previousSessions: TerminalSession[]
  gridColumns: number // 0 = auto, -1 = flexible (react-grid-layout)
  rowHeight: number
  flexibleLayouts: Record<string, FlexibleLayoutRect>
  sortMode: SortMode
  statusFilter: StatusFilter
  terminalOrder: string[]
  visibleTerminalIds: string[]
  focusableTerminalIds: string[]
  minimizedTerminals: Set<string>
  sessionDockCollapsed: boolean
  isOnboardingOpen: boolean
  diffSidebarTerminalId: string | null
  gitDiffStats: Map<string, GitDiffStat>
  rightPanelTab: PanelTab
  isDiffPanelMaximized: boolean
  diffPanelWidth: number
  mainViewMode: 'sessions' | 'tasks' | 'workflows'
  workflowsLandingTab: 'runs' | 'review'
  workflowsRunFilter: RunBucket
  workflowsRunsInflight: number
  workflowsRunsReloadToken: number
  /** Run selected in the Inbox list, shown in the detail pane. */
  selectedRunId: string | null
  selectedTaskId: string | null
  taskStatusFilter: TaskStatusFilter
  taskSourceFilter: TaskSourceFilter
  taskIncludeArchived: boolean
  isTaskDialogOpen: boolean
  taskDialogDefaultStatus: TaskStatus
  editingTask: TaskConfig | null
  activeTabId: string | null
  setActiveWorkspace: (id: string) => void
  setFocusedTerminal: (id: string | null) => void
  setSelectedTerminal: (id: string | null) => void
  setPreviewTerminal: (id: string | null) => void
  setRenamingTerminalId: (id: string | null) => void
  setSortMode: (mode: SortMode) => void
  setStatusFilter: (filter: StatusFilter) => void
  toggleSidebar: () => void
  setNewAgentDialogOpen: (open: boolean) => void
  setAddProjectDialogOpen: (open: boolean) => void
  setWorkflowEditorOpen: (open: boolean) => void
  setPendingWorkflowRun: (
    workflowId: string | null,
    context?: { task?: TaskConfig; source?: TerminalSession }
  ) => void
  setEditingWorkflowId: (id: string | null) => void
  setEditingProject: (project: ProjectConfig | null) => void
  setCommandPaletteOpen: (open: boolean) => void
  setShortcutsPanelOpen: (open: boolean) => void
  setSettingsOpen: (open: boolean) => void
  setSettingsCategory: (cat: SettingsCategory) => void
  setSessionBanner: (show: boolean, sessions?: TerminalSession[]) => void
  setGridColumns: (cols: number) => void
  setRowHeight: (height: number) => void
  setFlexibleLayouts: (layouts: Record<string, FlexibleLayoutRect>) => void
  setTerminalOrder: (order: string[]) => void
  setVisibleTerminalIds: (ids: string[]) => void
  setFocusableTerminalIds: (ids: string[]) => void
  reorderTerminals: (fromIndex: number, toIndex: number) => void
  toggleMinimized: (id: string) => void
  toggleSessionDockCollapsed: () => void
  setOnboardingOpen: (open: boolean) => void
  setDiffSidebarTerminalId: (id: string | null, tab?: PanelTab) => void
  updateGitDiffStat: (terminalId: string, stat: GitDiffStat) => void
  updateGitDiffStats: (stats: Map<string, GitDiffStat>) => void
  setRightPanelTab: (tab: PanelTab) => void
  setDiffPanelMaximized: (maximized: boolean) => void
  setDiffPanelWidth: (width: number) => void
  setMainViewMode: (mode: 'sessions' | 'tasks' | 'workflows') => void
  setWorkflowsLandingTab: (tab: 'runs' | 'review') => void
  setWorkflowsRunFilter: (filter: RunBucket) => void
  setSelectedRunId: (id: string | null) => void
  beginWorkflowsRunsLoad: () => void
  endWorkflowsRunsLoad: () => void
  bumpWorkflowsRunsReload: () => void
  setSelectedTaskId: (id: string | null) => void
  setTaskStatusFilter: (filter: TaskStatusFilter) => void
  setTaskSourceFilter: (filter: TaskSourceFilter) => void
  setTaskIncludeArchived: (include: boolean) => void
  setTaskDialogOpen: (open: boolean, defaultStatus?: TaskStatus) => void
  setEditingTask: (task: TaskConfig | null) => void
  setActiveTabId: (id: string | null) => void
  /** Live runs keyed by run id — one workflow can have several at once. */
  workflowExecutions: Map<string, WorkflowExecution>
  setWorkflowExecution: (runId: string, execution: WorkflowExecution) => void
  updateVersion: string | null
  setUpdateVersion: (version: string | null) => void
  worktreeCache: Map<string, WorktreeInfo[]>
  loadWorktrees: (projectPath: string, force?: boolean) => Promise<void>
  sidebarProjectSort: ProjectSortMode
  sidebarWorktreeSort: WorktreeSortMode
  sidebarWorktreeFilter: WorktreeFilter
  setSidebarProjectSort: (mode: ProjectSortMode) => void
  setSidebarWorktreeSort: (mode: WorktreeSortMode) => void
  setSidebarWorktreeFilter: (filter: WorktreeFilter) => void
  sidebarViewMode: SidebarViewMode
  setSidebarViewMode: (mode: SidebarViewMode) => void
  reorderProjects: (fromIndex: number, toIndex: number) => void
  sidebarWorkflowFilter: WorkflowFilter
  setSidebarWorkflowFilter: (filter: WorkflowFilter) => void
  reorderWorkflows: (fromIndex: number, toIndex: number) => void
}

export interface TasksSlice {
  getTasksForProject: (projectName: string) => TaskConfig[]
  getTaskQueue: (projectName: string) => TaskConfig[]
  getNextTask: (projectName: string) => TaskConfig | undefined
  addTask: (task: TaskConfig) => void
  removeTask: (id: string) => void
  updateTask: (id: string, updates: Partial<TaskConfig>) => void
  reorderTask: (id: string, newOrder: number) => void
  startTask: (id: string, sessionId: string, agentType: AiAgentType, worktreePath?: string) => void
  completeTask: (id: string) => void
  reviewTask: (id: string) => void
  cancelTask: (id: string) => void
  reopenTask: (id: string) => void
  archiveTask: (id: string) => void
  unarchiveTask: (id: string) => void
}

export type AppStore = TerminalsSlice & ProjectsSlice & UISlice & TasksSlice
