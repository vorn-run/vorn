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
  TaskStatus,
  MobileProject,
  UpdateStatus
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
export type PanelTab = 'changes'

export interface FlexibleLayoutRect {
  x: number
  y: number
  w: number
  h: number
}

/**
 * How one session card divides its interior.
 *
 * `terminal` is the fraction of the card's width the terminal keeps, the rest
 * going to the pane column. `panes` holds the fraction of that column's height
 * each stacked pane keeps, in render order — a pane with no entry falls back to
 * an even share, so opening a third pane never needs a migration.
 */
export interface CardSplit {
  terminal: number
  panes: number[]
}

/**
 * State of a file-editor pane. Independent of its session's tree pane — the
 * editor can be open, maximized, and closed on its own.
 */
export interface EditorPaneState {
  /** Absolute path of the open file. */
  filePath: string
  /**
   * The session this editor belongs to, which is not always its key.
   *
   * A session's own editor is stored under the session id, so every existing
   * `editorPanes.get(sessionId)` still reads it. A file popped out to a card of
   * its own is stored under a `card:` id instead, and this is the only thing
   * that still says whose file it is — which worktree to read it from, whose
   * branch to label it with, and what to tear it down alongside.
   */
  sessionId: string
}

/** True when a pane entry was popped out of its session's card. */
export function isPromotedPane(paneId: string, pane: { sessionId: string }): boolean {
  return paneId !== pane.sessionId
}

/**
 * State of a session's browser pane. One per session, so a session can keep a
 * dev server or a doc page beside its agent.
 *
 * A pane holds several tabs, like any browser — a dev server and the docs you
 * are reading against it are the common pair, and forcing one to replace the
 * other made the pane far less useful than the space it costs.
 */
/**
 * One tab, split by who is speaking.
 *
 * `url` is intent — what the pane was *told* to show — and is what drives the
 * guest's `src`. `liveUrl` and `title` are observation: where the guest
 * actually ended up and what it calls itself, which only it can report.
 *
 * They are separate fields rather than one because `src` is bound to `url`.
 * Writing an observed url back into it re-sets `src` to a page the guest is
 * already on, and the reload that follows throws away the scroll position and
 * any half-typed form — on every navigation. Keeping intent stable means the
 * label can follow the guest anywhere without touching it.
 */
export interface BrowserTabState {
  /** Normalized absolute url the pane was asked to load. Drives the guest's `src`. */
  url: string
  /** Where the guest actually is, once it reports. Absent until it does. */
  liveUrl?: string
  /** The guest's own title, once it reports one. */
  title?: string
}

export interface BrowserPaneState {
  /** One entry per tab. Never empty while the pane exists. */
  tabs: BrowserTabState[]
  /** Index into `tabs`. Always in range. */
  activeTab: number
  /** The session this browser belongs to. See `EditorPaneState.sessionId`. */
  sessionId: string
}

/** Where a tab actually is: what the guest reports, or the intent until it does. */
export function tabUrl(tab: BrowserTabState): string {
  return tab.liveUrl ?? tab.url
}

/**
 * State of a session's terminals panel — the shells it holds beside its agent.
 *
 * Deliberately the same shape as `BrowserPaneState`, because it is the same
 * problem: an ordered list with one of them in front, whose presence *is* the
 * pane being open.
 *
 * What it holds are session ids. A shell terminal is already a full session
 * with its own pid, so a panel needs no new entity — only a record of which
 * session each shell hangs under, which is this list. Taking an id out of it is
 * the whole of "extract this terminal": it is a session already, so it becomes
 * a grid cell, a tab, a sidebar row and a focus target the moment it is no
 * longer claimed here.
 */
export interface TerminalsPaneState {
  /** Session ids of the shells in this panel. Never empty while it exists. */
  terminals: string[]
  /** Index into `terminals`. Always in range. */
  activeTab: number
}

/**
 * Which tab a panel is showing, clamped into range.
 *
 * `activeTab` is kept in range by every action that writes it and by the
 * persisted-state reader, so this is belt and braces — but it is the one place
 * that answers the question, and the card indexes its tab list by it. Two
 * answers, one clamped and one not, is how an out-of-range index reaches a
 * dereference.
 */
export function activePanelIndex(pane: TerminalsPaneState | undefined): number {
  if (!pane || pane.terminals.length === 0) return 0
  return Math.min(Math.max(pane.activeTab, 0), pane.terminals.length - 1)
}

/** The terminal a panel is currently showing. */
export function activePanelTerminalId(pane: TerminalsPaneState | undefined): string | null {
  return pane ? (pane.terminals[activePanelIndex(pane)] ?? null) : null
}

/**
 * The page a browser pane is currently showing.
 *
 * Where the guest actually is, not where it was sent — these differ whenever
 * the page redirected, followed a link, or was navigated by an agent, and the
 * label has to name the page in front of the person.
 */
export function activeBrowserUrl(pane: BrowserPaneState | undefined): string | null {
  const tab = pane?.tabs[pane.activeTab]
  return tab ? tabUrl(tab) : null
}

/**
 * State of a session's device pane — the simulator that session has claimed.
 *
 * Deliberately not persisted, unlike files/editor/browser panes. A claim lives
 * in the main process and does not survive a restart, so reviving this pane
 * from disk would show a frame for a device this session no longer holds.
 */
export interface DevicePaneState {
  /** UDID of the claimed simulator this pane is showing. */
  udid: string
  /** Display name, for the pane title. */
  name: string
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
  | 'updates'
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
  /** How each session card divides its interior, keyed by session id. */
  cardSplits: Record<string, CardSplit>
  sortMode: SortMode
  statusFilter: StatusFilter
  terminalOrder: string[]
  visibleTerminalIds: string[]
  focusableTerminalIds: string[]
  minimizedTerminals: Set<string>
  /** Session ids whose file-tree pane is open. Keyed by owner, one per session. */
  filesPanes: Set<string>
  /**
   * Pane id → the file that editor is showing.
   *
   * Keyed by pane, not by session, so one session can have several: its own
   * editor under the session id, plus a `card:` entry for every file popped out
   * to a card of its own. An entry whose key is not its `sessionId` is exactly
   * what a promoted card is — there is no second flag to fall out of step with
   * it, and closing the pane is what removes the card.
   */
  editorPanes: Map<string, EditorPaneState>
  /** Pane id → the pages that browser is showing. Keyed like `editorPanes`. */
  browserPanes: Map<string, BrowserPaneState>
  /**
   * What a closed browser pane was showing, so reopening restores the tabs
   * rather than starting over.
   *
   * `browserPanes` does double duty — an entry's presence is what makes the
   * pane open, and its value is the tabs — so closing has to delete the entry
   * and would otherwise take the tabs with it. Kept separate rather than adding
   * an `open` flag so every `browserPanes.has(...)` check reads the same.
   */
  browserMemory: Map<string, BrowserPaneState>
  /** Session id → the simulator its device pane is showing. One device per session. */
  devicePanes: Map<string, DevicePaneState>
  /**
   * Session id → the shells it holds beside its agent. One panel per session.
   *
   * A terminal listed here is claimed by that session: it is drawn in the panel
   * and nowhere else, and is deliberately absent from the grid, the tab strip,
   * the sidebar, the dock and keyboard nav until it is extracted. That hiding
   * is what keeps a terminal to a single rendered slot — the registry is
   * last-writer-wins, so two slots for one id would fight over the wrapper.
   */
  terminalsPanes: Map<string, TerminalsPaneState>
  /**
   * Pane id currently maximized, or null. At most one app-wide. A maximized
   * pane covers only its owner session's footprint — other sessions are
   * unaffected, which is what makes it usable for side-by-side comparison.
   */
  maximizedPaneId: string | null
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
  /**
   * Commit one session card's interior split. Called on pointerup only — the
   * live drag drives local state, so a resize writes to storage once.
   */
  setCardSplit: (sessionId: string, split: CardSplit) => void
  setTerminalOrder: (order: string[]) => void
  setVisibleTerminalIds: (ids: string[]) => void
  setFocusableTerminalIds: (ids: string[]) => void
  /**
   * Move `draggedId` to where `droppedOnId` sits in the session order.
   *
   * Takes ids rather than indices because the lists callers drag within
   * interleave popped-out cards, and `terminalOrder` holds sessions only.
   * Dragging a card is a no-op — a card has no position of its own, it is drawn
   * beside its owner — and dropping onto one targets that owner's slot.
   */
  reorderTerminals: (draggedId: string, droppedOnId: string) => void
  toggleMinimized: (id: string) => void
  /** Open (or focus) the file-tree pane owned by `sessionId`. */
  openFilesPane: (sessionId: string) => void
  closeFilesPane: (sessionId: string) => void
  toggleFilesPane: (sessionId: string) => void
  /**
   * Show `filePath` in the session's editor pane, creating it if needed.
   * Independent of the tree pane — the editor works with Files closed.
   */
  openEditorPane: (sessionId: string, filePath: string) => void
  closeEditorPane: (paneId: string) => void
  /**
   * Show `url` in the session's browser pane, creating it if needed.
   *
   * Accepts whatever the user typed — it is normalized here, so `localhost:5173`
   * works. A url that cannot be loaded (a refused scheme, or nonsense) leaves
   * the pane on its current page rather than blanking it. Omitting `url`
   * reveals the session's browser without changing the page it is showing.
   */
  /**
   * Show a session's browser, optionally at a url.
   *
   * `trusted` marks a url main has already vetted — it decided the scheme was
   * allowed *and*, for `file:`, that the path is inside the session's root.
   * The renderer cannot redo the second half: containment is a filesystem
   * question and it has no filesystem. Re-normalizing here would quietly drop
   * a `file:` url main just approved, so a trusted url is taken as given.
   */
  openBrowserPane: (sessionId: string, url?: string, opts?: { trusted?: boolean }) => void
  closeBrowserPane: (paneId: string) => void
  toggleBrowserPane: (sessionId: string) => void
  /** Add a tab to the browser and make it active. See `openBrowserPane` for `trusted`. */
  addBrowserTab: (paneId: string, url?: string, opts?: { trusted?: boolean }) => void
  /**
   * Close one tab. Closing the last one closes the pane, since a browser with
   * no page is just an empty box.
   */
  closeBrowserTab: (paneId: string, index: number) => void
  setActiveBrowserTab: (paneId: string, index: number) => void
  /**
   * Record where a guest actually went, and what it calls itself.
   *
   * Reported by the `<webview>` rather than asked for: a page redirects, a
   * link is followed, an agent navigates over CDP. None of those pass through
   * `openBrowserPane`, so without this the strip keeps naming the page the tab
   * was originally sent to — a label describing something nobody is looking at.
   *
   * Deliberately separate from the navigation actions. It writes `liveUrl` and
   * `title`, never `url`, so the guest's own `src` binding is untouched and
   * observing a navigation cannot cause one.
   */
  syncBrowserTab: (paneId: string, index: number, seen: { url?: string; title?: string }) => void
  /**
   * Open the pane for a device main already holds. Used when main itself asks
   * for the pane (the agent claimed the device), where the claim is a
   * precondition rather than something the renderer arranges.
   */
  openDevicePane: (sessionId: string, device: DevicePaneState) => void
  /**
   * Claim the device, then open the pane — the path for a person picking from
   * the picker. Opening without claiming leaves the pane polling a session
   * main has no device for, so every frame fails with "No device is claimed"
   * and the picker appears to have done nothing. Returns the failure message
   * so contention can be shown where the person is looking.
   */
  claimAndOpenDevicePane: (sessionId: string, device: DevicePaneState) => Promise<string | null>
  closeDevicePane: (sessionId: string) => void
  /**
   * Show the session's terminals panel, adding `terminalId` to it if given.
   *
   * Opening with nothing to show would be an empty box taking up a pane, so the
   * caller creates a shell first and hands it here.
   */
  openTerminalsPane: (sessionId: string, terminalId: string) => void
  /** Close the panel. The shells in it are the caller's to dispose of. */
  closeTerminalsPane: (sessionId: string) => void
  setActivePanelTerminal: (sessionId: string, index: number) => void
  /**
   * Take a terminal out of its session's panel and let it stand on its own.
   *
   * Only removes the claim — the terminal is already a session, so it has a
   * grid cell, a tab, a sidebar row and a focus slot the moment nothing is
   * hiding it. Taking the last one closes the panel, since a panel with no
   * shells is a box taking up space.
   */
  extractPanelTerminal: (sessionId: string, terminalId: string) => void
  /** Maximize a pane over its owner session's footprint, or null to restore. */
  setMaximizedPane: (paneId: string | null) => void
  /**
   * Open `filePath` as a card of its own rather than in the session's editor.
   *
   * Several files can be open this way at once, which is the point: the
   * session's own editor holds exactly one, so reading two files side by side
   * was not possible without this. Returns the new card's id.
   */
  promoteFile: (sessionId: string, filePath: string) => string
  /**
   * Take one tab out of a browser pane and give it a card of its own.
   *
   * The tab leaves the strip rather than being copied — two views of one url
   * would be two guests loading the same page, each with its own scroll
   * position and half-filled forms. Returns the new card's id, or null if the
   * index names no tab.
   */
  promoteBrowserTab: (paneId: string, index: number) => string | null
  /**
   * Put a promoted card back where it came from: the file into its session's
   * editor, the tab onto the end of its session's tab strip.
   *
   * Where the session has no such pane open, this opens one — the card has to
   * land somewhere, and refusing to return it would strand it.
   */
  returnCardToSession: (cardId: string) => void
  /**
   * Close a popped-out card, asking first if it holds unsaved edits.
   *
   * One action rather than a `closeEditorPane` / `closeBrowserPane` fork at each
   * button, because the fork was got wrong: the card's own ✕ confirmed, while
   * its tab and its sidebar row discarded the buffer without a word.
   */
  closeCard: (cardId: string) => void
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
  /**
   * Where the app updater is, as one value pushed from main. Named for the app
   * rather than plain `updateStatus` because TerminalsSlice already owns that
   * name for per-session agent status, and the two share a store.
   *
   * Dismissal is tracked separately so hiding the banner never destroys the
   * fact that an update is staged — the old code nulled the version and lost it
   * until relaunch.
   */
  appUpdateStatus: UpdateStatus
  setAppUpdateStatus: (status: UpdateStatus) => void
  updateBannerDismissed: boolean
  setUpdateBannerDismissed: (dismissed: boolean) => void
  worktreeCache: Map<string, WorktreeInfo[]>
  loadWorktrees: (projectPath: string, force?: boolean) => Promise<void>

  /**
   * Per-project answer to "is this a mobile app", used only to decide whether
   * to offer the device control. Derived and cheap to recompute, so it is
   * deliberately kept out of the persisted pane state: a stale "not mobile"
   * written to localStorage would hide the button after someone adds Expo to a
   * project and never come back.
   */
  mobileProjectCache: Map<string, MobileProject>
  loadMobileProject: (projectPath: string, force?: boolean) => Promise<void>
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
