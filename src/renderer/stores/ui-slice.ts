import { StateCreator } from 'zustand'
import { TerminalSession } from '../../shared/types'
import {
  AppStore,
  UISlice,
  SidebarViewMode,
  FlexibleLayoutRect,
  TaskSourceFilter,
  EditorPaneState,
  BrowserPaneState
} from './types'
import { filesPaneId, editorPaneId, browserPaneId } from '../lib/pane-id'
import { normalizeUrl } from '../lib/browser-url'

const EMPTY_SESSIONS: TerminalSession[] = []
const WORKTREE_CACHE_TTL = 5_000
const worktreeCacheTimestamps = new Map<string, number>()
const GRID_STORAGE_KEY = 'vorn:gridSettings'
const SIDEBAR_STORAGE_KEY = 'vorn:sidebarSettings'
const FLEXIBLE_STORAGE_KEY = 'vorn:flexibleLayouts'
const PANES_STORAGE_KEY = 'vorn:panes'
/**
 * Where a browser pane opened with no url starts. Deliberately blank: guessing
 * a page would be wrong more often than not, and the address bar is focused
 * and empty, which is the prompt to type one.
 */
const DEFAULT_BROWSER_URL = 'about:blank'

function loadGridSettings(): { gridColumns?: number; sortMode?: string; statusFilter?: string } {
  try {
    const raw = localStorage.getItem(GRID_STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function saveGridSettings(patch: Record<string, unknown>): void {
  try {
    const current = loadGridSettings()
    localStorage.setItem(GRID_STORAGE_KEY, JSON.stringify({ ...current, ...patch }))
  } catch {
    /* ignore */
  }
}

function loadFlexibleLayouts(): Record<string, FlexibleLayoutRect> {
  try {
    const raw = localStorage.getItem(FLEXIBLE_STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function saveFlexibleLayouts(layouts: Record<string, FlexibleLayoutRect>): void {
  try {
    localStorage.setItem(FLEXIBLE_STORAGE_KEY, JSON.stringify(layouts))
  } catch {
    /* ignore */
  }
}

/**
 * Open child panes, persisted so a reload restores the same workspace.
 *
 * Shape: `{ files: sessionId[], editors: { [id]: filePath }, browsers: { [id]: url } }`.
 * Entries whose session no longer exists are pruned by `removeTerminal` when it
 * closes, and by `reconcilePanes` for sessions that never came back.
 */
function loadPanes(): {
  filesPanes: Set<string>
  editorPanes: Map<string, EditorPaneState>
  browserPanes: Map<string, BrowserPaneState>
} {
  try {
    const raw = localStorage.getItem(PANES_STORAGE_KEY)
    if (!raw) return { filesPanes: new Set(), editorPanes: new Map(), browserPanes: new Map() }
    const parsed = JSON.parse(raw) as {
      files?: string[]
      editors?: Record<string, string>
      browsers?: Record<string, string | { tabs: string[]; activeTab: number }>
    }
    return {
      filesPanes: new Set(parsed.files ?? []),
      editorPanes: new Map(
        Object.entries(parsed.editors ?? {}).map(([id, filePath]) => [id, { filePath }])
      ),
      browserPanes: parsePersistedBrowsers(parsed.browsers)
    }
  } catch {
    return { filesPanes: new Set(), editorPanes: new Map(), browserPanes: new Map() }
  }
}

/**
 * Read the persisted browser panes.
 *
 * Older builds stored a single url string per session, before a pane could
 * hold tabs. Those are read as a one-tab pane rather than discarded, so
 * upgrading doesn't silently close the page someone had open.
 */
export function parsePersistedBrowsers(
  saved: Record<string, string | { tabs?: string[]; activeTab?: number }> | undefined
): Map<string, BrowserPaneState> {
  return new Map(
    Object.entries(saved ?? {}).map(([id, entry]) => {
      if (typeof entry === 'string') return [id, { tabs: [entry], activeTab: 0 }]
      const tabs = entry.tabs?.length ? entry.tabs : [DEFAULT_BROWSER_URL]
      const activeTab = Math.min(Math.max(entry.activeTab ?? 0, 0), tabs.length - 1)
      return [id, { tabs, activeTab }]
    })
  )
}

function savePanes(
  filesPanes: Set<string>,
  editorPanes: Map<string, EditorPaneState>,
  browserPanes: Map<string, BrowserPaneState>
): void {
  try {
    localStorage.setItem(
      PANES_STORAGE_KEY,
      JSON.stringify({
        files: [...filesPanes],
        editors: Object.fromEntries([...editorPanes].map(([id, s]) => [id, s.filePath])),
        browsers: Object.fromEntries(
          [...browserPanes].map(([id, s]) => [id, { tabs: s.tabs, activeTab: s.activeTab }])
        )
      })
    )
  } catch {
    /* ignore */
  }
}

/**
 * Drop persisted pane entries whose session no longer exists.
 *
 * `removeTerminal` prunes sessions closed during a run, but a session that
 * simply never came back after a restart leaves its entry behind. Reconciling
 * against the live set keeps localStorage from growing without bound and stops
 * a recycled id from inheriting a stale pane.
 */
function reconcilePanes(
  filesPanes: Set<string>,
  editorPanes: Map<string, EditorPaneState>,
  browserPanes: Map<string, BrowserPaneState>,
  liveSessionIds: Set<string>
): {
  filesPanes: Set<string>
  editorPanes: Map<string, EditorPaneState>
  browserPanes: Map<string, BrowserPaneState>
} | null {
  const nextFiles = new Set([...filesPanes].filter((id) => liveSessionIds.has(id)))
  const nextEditors = new Map([...editorPanes].filter(([id]) => liveSessionIds.has(id)))
  const nextBrowsers = new Map([...browserPanes].filter(([id]) => liveSessionIds.has(id)))
  if (
    nextFiles.size === filesPanes.size &&
    nextEditors.size === editorPanes.size &&
    nextBrowsers.size === browserPanes.size
  ) {
    return null
  }
  savePanes(nextFiles, nextEditors, nextBrowsers)
  return { filesPanes: nextFiles, editorPanes: nextEditors, browserPanes: nextBrowsers }
}

function loadSidebarSettings(): Record<string, string> {
  try {
    const raw = localStorage.getItem(SIDEBAR_STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function saveSidebarSettings(patch: Record<string, unknown>): void {
  try {
    const current = loadSidebarSettings()
    localStorage.setItem(SIDEBAR_STORAGE_KEY, JSON.stringify({ ...current, ...patch }))
  } catch {
    /* ignore */
  }
}

const savedGrid = loadGridSettings()
const savedSidebar = loadSidebarSettings()

export const createUISlice: StateCreator<AppStore, [], [], UISlice> = (set, get) => ({
  activeWorkspace: 'personal',
  focusedTerminalId: null,
  selectedTerminalId: null,
  previewTerminalId: null,
  renamingTerminalId: null,
  isSidebarOpen: true,
  isNewAgentDialogOpen: false,
  isAddProjectDialogOpen: false,
  isWorkflowEditorOpen: false,
  editingWorkflowId: null,
  pendingWorkflowRun: null,
  editingProject: null,
  isCommandPaletteOpen: false,
  isShortcutsPanelOpen: false,
  isSettingsOpen: false,
  settingsCategory: 'appearance',
  showSessionBanner: false,
  previousSessions: [],
  gridColumns: (savedGrid.gridColumns as number) ?? 0,
  rowHeight: 208,
  flexibleLayouts: loadFlexibleLayouts(),
  sortMode: (savedGrid.sortMode as 'manual' | 'created' | 'recent') ?? 'manual',
  statusFilter:
    (savedGrid.statusFilter as 'all' | 'running' | 'waiting' | 'idle' | 'error') ?? 'all',
  terminalOrder: [],
  visibleTerminalIds: [],
  focusableTerminalIds: [],
  minimizedTerminals: new Set(),
  ...loadPanes(),
  maximizedPaneId: null,
  sessionDockCollapsed: false,
  isOnboardingOpen: false,
  diffSidebarTerminalId: null,
  gitDiffStats: new Map(),
  rightPanelTab: 'changes',
  isDiffPanelMaximized: false,
  diffPanelWidth: 480,
  mainViewMode: 'sessions' as const,
  workflowsLandingTab: 'runs' as const,
  workflowsRunFilter: 'all' as const,
  workflowsRunsInflight: 0,
  workflowsRunsReloadToken: 0,
  selectedRunId: null,
  selectedTaskId: null,
  taskStatusFilter: 'all' as const,
  taskSourceFilter: 'all' as TaskSourceFilter,
  taskIncludeArchived: false,
  isTaskDialogOpen: false,
  taskDialogDefaultStatus: 'todo' as const,
  editingTask: null,
  activeTabId: null,

  setActiveWorkspace: (id) => {
    const config = get().config
    if (config) {
      const updated = { ...config, defaults: { ...config.defaults, activeWorkspace: id } }
      window.api.saveConfig(updated)
      set({ activeWorkspace: id, activeProject: null, config: updated })
    } else {
      set({ activeWorkspace: id, activeProject: null })
    }
  },
  setFocusedTerminal: (id) =>
    set(() => ({
      focusedTerminalId: id,
      selectedTerminalId: id,
      previewTerminalId: null
    })),
  setSelectedTerminal: (id) => set({ selectedTerminalId: id }),
  setPreviewTerminal: (id) => set({ previewTerminalId: id }),
  setRenamingTerminalId: (id) => set({ renamingTerminalId: id }),
  setSortMode: (mode) => {
    saveGridSettings({ sortMode: mode })
    set({ sortMode: mode })
  },
  setStatusFilter: (filter) => {
    saveGridSettings({ statusFilter: filter })
    set({ statusFilter: filter })
  },

  toggleSidebar: () => set((state) => ({ isSidebarOpen: !state.isSidebarOpen })),

  setNewAgentDialogOpen: (open) => set({ isNewAgentDialogOpen: open }),

  setAddProjectDialogOpen: (open) => set({ isAddProjectDialogOpen: open }),

  setWorkflowEditorOpen: (open) => set({ isWorkflowEditorOpen: open }),

  setPendingWorkflowRun: (workflowId, context) =>
    set({ pendingWorkflowRun: workflowId ? { workflowId, context } : null }),

  setEditingWorkflowId: (id) => set({ editingWorkflowId: id }),

  setEditingProject: (project) => set({ editingProject: project }),

  setCommandPaletteOpen: (open) => set({ isCommandPaletteOpen: open }),
  setShortcutsPanelOpen: (open) => set({ isShortcutsPanelOpen: open }),

  setSettingsOpen: (open) => set({ isSettingsOpen: open }),

  setSettingsCategory: (cat) => set({ settingsCategory: cat }),

  setSessionBanner: (show, sessions) =>
    set({
      showSessionBanner: show,
      previousSessions: sessions ?? EMPTY_SESSIONS
    }),

  setGridColumns: (cols) => {
    saveGridSettings({ gridColumns: cols })
    set({ gridColumns: cols })
  },

  setRowHeight: (height) => {
    const config = get().config
    if (config) {
      const updated = { ...config, defaults: { ...config.defaults, rowHeight: height } }
      window.api.saveConfig(updated)
      set({ rowHeight: height, config: updated })
    } else {
      set({ rowHeight: height })
    }
  },

  setFlexibleLayouts: (layouts) => {
    saveFlexibleLayouts(layouts)
    set({ flexibleLayouts: layouts })
  },

  setTerminalOrder: (order) => set({ terminalOrder: order }),
  setVisibleTerminalIds: (ids) =>
    set((state) => {
      // Sessions restore by their persisted id, so pane entries stay valid across
      // restarts — but a session that never comes back would leave its entry
      // behind forever. Reconcile against the live set as it settles.
      const live = new Set(state.terminals.keys())
      const reconciled =
        live.size > 0
          ? reconcilePanes(state.filesPanes, state.editorPanes, state.browserPanes, live)
          : null
      return { visibleTerminalIds: ids, ...(reconciled ?? {}) }
    }),
  setFocusableTerminalIds: (ids) => set({ focusableTerminalIds: ids }),

  reorderTerminals: (fromIndex, toIndex) =>
    set((state) => {
      const order = [...state.terminalOrder]
      const [moved] = order.splice(fromIndex, 1)
      order.splice(toIndex, 0, moved)
      window.api.reorderSessions(order)
      return { terminalOrder: order }
    }),

  toggleMinimized: (id) =>
    set((state) => {
      const next = new Set(state.minimizedTerminals)
      const nowMinimized = !next.has(id)
      if (nowMinimized) next.add(id)
      else next.delete(id)
      // A pane that just got minimized can't stay maximized.
      const clearMax = nowMinimized && state.maximizedPaneId === id
      return { minimizedTerminals: next, ...(clearMax ? { maximizedPaneId: null } : {}) }
    }),

  openFilesPane: (sessionId) =>
    set((state) => {
      const paneId = filesPaneId(sessionId)
      // Opening an already-open pane focuses it, which for a minimized pane
      // means bringing it back rather than doing nothing.
      if (state.filesPanes.has(sessionId)) {
        if (!state.minimizedTerminals.has(paneId)) return {}
        const minimized = new Set(state.minimizedTerminals)
        minimized.delete(paneId)
        return { minimizedTerminals: minimized }
      }
      const next = new Set(state.filesPanes)
      next.add(sessionId)
      savePanes(next, state.editorPanes, state.browserPanes)
      return { filesPanes: next }
    }),

  closeFilesPane: (sessionId) =>
    set((state) => {
      if (!state.filesPanes.has(sessionId)) return {}
      const next = new Set(state.filesPanes)
      next.delete(sessionId)
      savePanes(next, state.editorPanes, state.browserPanes)
      const paneId = filesPaneId(sessionId)
      const minimized = new Set(state.minimizedTerminals)
      minimized.delete(paneId)
      return {
        filesPanes: next,
        minimizedTerminals: minimized,
        ...(state.maximizedPaneId === paneId ? { maximizedPaneId: null } : {})
      }
    }),

  toggleFilesPane: (sessionId) => {
    const { filesPanes, minimizedTerminals, openFilesPane, closeFilesPane } = get()
    // A minimized pane is open but out of sight, so the toggle restores it
    // instead of closing something the user cannot currently see.
    const isHidden = minimizedTerminals.has(filesPaneId(sessionId))
    if (filesPanes.has(sessionId) && !isHidden) closeFilesPane(sessionId)
    else openFilesPane(sessionId)
  },

  openEditorPane: (sessionId, filePath) =>
    set((state) => {
      const next = new Map(state.editorPanes)
      next.set(sessionId, { filePath })
      savePanes(state.filesPanes, next, state.browserPanes)
      // Re-opening into a minimized editor should surface it again.
      const paneId = editorPaneId(sessionId)
      const minimized = new Set(state.minimizedTerminals)
      minimized.delete(paneId)
      return { editorPanes: next, minimizedTerminals: minimized }
    }),

  closeEditorPane: (sessionId) =>
    set((state) => {
      if (!state.editorPanes.has(sessionId)) return {}
      const next = new Map(state.editorPanes)
      next.delete(sessionId)
      savePanes(state.filesPanes, next, state.browserPanes)
      const paneId = editorPaneId(sessionId)
      const minimized = new Set(state.minimizedTerminals)
      minimized.delete(paneId)
      return {
        editorPanes: next,
        minimizedTerminals: minimized,
        ...(state.maximizedPaneId === paneId ? { maximizedPaneId: null } : {})
      }
    }),

  openBrowserPane: (sessionId, url) =>
    set((state) => {
      const paneId = browserPaneId(sessionId)
      const minimized = new Set(state.minimizedTerminals)
      const wasHidden = minimized.delete(paneId)

      // No url means "show me this session's browser" — keep whatever page it
      // already had rather than resetting it to blank.
      const next = new Map(state.browserPanes)
      const existing = next.get(sessionId)
      if (url !== undefined) {
        const normalized = normalizeUrl(url)
        if (!normalized) return wasHidden ? { minimizedTerminals: minimized } : {}
        if (existing) {
          // Navigating replaces the page in the tab the user is looking at,
          // the way an address bar does — it does not spawn a tab.
          const tabs = [...existing.tabs]
          tabs[existing.activeTab] = normalized
          next.set(sessionId, { ...existing, tabs })
        } else {
          next.set(sessionId, { tabs: [normalized], activeTab: 0 })
        }
      } else if (!existing) {
        next.set(sessionId, { tabs: [DEFAULT_BROWSER_URL], activeTab: 0 })
      } else if (!wasHidden) {
        return {}
      }

      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next, minimizedTerminals: minimized }
    }),

  closeBrowserPane: (sessionId) =>
    set((state) => {
      if (!state.browserPanes.has(sessionId)) return {}
      const next = new Map(state.browserPanes)
      next.delete(sessionId)
      savePanes(state.filesPanes, state.editorPanes, next)
      const paneId = browserPaneId(sessionId)
      const minimized = new Set(state.minimizedTerminals)
      minimized.delete(paneId)
      return {
        browserPanes: next,
        minimizedTerminals: minimized,
        ...(state.maximizedPaneId === paneId ? { maximizedPaneId: null } : {})
      }
    }),

  toggleBrowserPane: (sessionId) => {
    const { browserPanes, minimizedTerminals, openBrowserPane, closeBrowserPane } = get()
    // A minimized pane is open but out of sight, so the toggle restores it
    // instead of closing something the user cannot currently see.
    const isHidden = minimizedTerminals.has(browserPaneId(sessionId))
    if (browserPanes.has(sessionId) && !isHidden) closeBrowserPane(sessionId)
    else openBrowserPane(sessionId)
  },

  addBrowserTab: (sessionId, url) =>
    set((state) => {
      const existing = state.browserPanes.get(sessionId)
      if (!existing) return {}
      const normalized = url === undefined ? DEFAULT_BROWSER_URL : normalizeUrl(url)
      if (!normalized) return {}
      const next = new Map(state.browserPanes)
      const tabs = [...existing.tabs, normalized]
      next.set(sessionId, { tabs, activeTab: tabs.length - 1 })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  closeBrowserTab: (sessionId, index) => {
    const existing = get().browserPanes.get(sessionId)
    if (!existing || index < 0 || index >= existing.tabs.length) return
    // The last tab going means the pane itself goes; an empty browser is just
    // a box taking up a grid cell.
    if (existing.tabs.length === 1) {
      get().closeBrowserPane(sessionId)
      return
    }
    set((state) => {
      const pane = state.browserPanes.get(sessionId)
      if (!pane) return {}
      const tabs = pane.tabs.filter((_, i) => i !== index)
      // Closing a tab left of the active one shifts it; closing the active one
      // lands on its neighbour rather than jumping to the far end.
      const activeTab = Math.min(
        index <= pane.activeTab ? pane.activeTab - 1 : pane.activeTab,
        tabs.length - 1
      )
      const next = new Map(state.browserPanes)
      next.set(sessionId, { tabs, activeTab: Math.max(activeTab, 0) })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    })
  },

  setActiveBrowserTab: (sessionId, index) =>
    set((state) => {
      const existing = state.browserPanes.get(sessionId)
      if (!existing || index < 0 || index >= existing.tabs.length) return {}
      if (existing.activeTab === index) return {}
      const next = new Map(state.browserPanes)
      next.set(sessionId, { ...existing, activeTab: index })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  setMaximizedPane: (paneId) =>
    set((state) => (state.maximizedPaneId === paneId ? {} : { maximizedPaneId: paneId })),

  toggleSessionDockCollapsed: () =>
    set((state) => ({ sessionDockCollapsed: !state.sessionDockCollapsed })),

  setOnboardingOpen: (open) => set({ isOnboardingOpen: open }),
  setDiffSidebarTerminalId: (id, tab) =>
    set({
      diffSidebarTerminalId: id,
      rightPanelTab: tab ?? 'changes',
      isDiffPanelMaximized: false
    }),
  setRightPanelTab: (tab) => set({ rightPanelTab: tab }),
  setDiffPanelMaximized: (maximized) => set({ isDiffPanelMaximized: maximized }),
  setDiffPanelWidth: (width) => set({ diffPanelWidth: width }),

  updateGitDiffStat: (terminalId, stat) =>
    set((state) => {
      const next = new Map(state.gitDiffStats)
      next.set(terminalId, stat)
      return { gitDiffStats: next }
    }),

  updateGitDiffStats: (stats) =>
    set((state) => {
      const next = new Map(state.gitDiffStats)
      for (const [id, stat] of stats) {
        next.set(id, stat)
      }
      return { gitDiffStats: next }
    }),

  setMainViewMode: (mode) => {
    const state = get()
    const config = state.config
    // mainViewMode is sourced from config.defaults (see ProjectSidebar/SidebarHeader);
    // the store field isn't synced on setConfig, so compare against config first.
    const prevMode = config?.defaults?.mainViewMode ?? state.mainViewMode
    if (prevMode === mode) return
    const extra: Record<string, unknown> =
      mode === 'sessions'
        ? {}
        : { diffSidebarTerminalId: null, focusedTerminalId: null, previewTerminalId: null }
    // Preserve editingWorkflowId so the selection survives tab switches,
    // but close the modal editor when leaving the workflows tab.
    if (prevMode === 'workflows' && mode !== 'workflows') {
      extra.isWorkflowEditorOpen = false
    }
    if (config) {
      const updated = { ...config, defaults: { ...config.defaults, mainViewMode: mode } }
      window.api.saveConfig(updated)
      set({ mainViewMode: mode, config: updated, ...extra })
    } else {
      set({ mainViewMode: mode, ...extra })
    }
  },
  setWorkflowsLandingTab: (tab) => set({ workflowsLandingTab: tab }),
  setWorkflowsRunFilter: (filter) => set({ workflowsRunFilter: filter }),
  setSelectedRunId: (id) => set({ selectedRunId: id }),
  beginWorkflowsRunsLoad: () =>
    set((s) => ({ workflowsRunsInflight: s.workflowsRunsInflight + 1 })),
  endWorkflowsRunsLoad: () =>
    set((s) => ({ workflowsRunsInflight: Math.max(0, s.workflowsRunsInflight - 1) })),
  bumpWorkflowsRunsReload: () =>
    set((s) => ({ workflowsRunsReloadToken: s.workflowsRunsReloadToken + 1 })),
  setSelectedTaskId: (id) => set({ selectedTaskId: id }),
  setTaskStatusFilter: (filter) => set({ taskStatusFilter: filter }),
  setTaskSourceFilter: (filter) => set({ taskSourceFilter: filter }),
  setTaskIncludeArchived: (include) => set({ taskIncludeArchived: include }),
  setTaskDialogOpen: (open, defaultStatus) =>
    set({ isTaskDialogOpen: open, taskDialogDefaultStatus: defaultStatus ?? 'todo' }),
  setEditingTask: (task) => set({ editingTask: task }),

  setActiveTabId: (id) => set({ activeTabId: id }),

  workflowExecutions: new Map(),
  setWorkflowExecution: (runId, execution) =>
    set((state) => {
      const next = new Map(state.workflowExecutions)
      next.set(runId, execution)
      return { workflowExecutions: next }
    }),

  updateVersion: null,
  setUpdateVersion: (version) => set({ updateVersion: version }),

  worktreeCache: new Map(),
  loadWorktrees: async (projectPath, force) => {
    if (!force) {
      const lastLoaded = worktreeCacheTimestamps.get(projectPath)
      if (lastLoaded && Date.now() - lastLoaded < WORKTREE_CACHE_TTL) return
    }
    worktreeCacheTimestamps.set(projectPath, Date.now())

    try {
      const worktrees = await window.api.listWorktrees(projectPath)
      const terminals = get().terminals

      const enriched = await Promise.all(
        worktrees.map(async (wt) => {
          if (wt.isMain) {
            return { ...wt, isDirty: false, diffStat: undefined, linkedSessionId: undefined }
          }
          const isDirty = await window.api.isWorktreeDirty(wt.path)
          const diffStat = isDirty
            ? ((await window.api.getGitDiffStat(wt.path)) ?? undefined)
            : undefined
          let linkedSessionId: string | undefined
          for (const [id, t] of terminals) {
            if (t.session.worktreePath === wt.path) {
              linkedSessionId = id
              break
            }
          }
          return { ...wt, isDirty, diffStat, linkedSessionId }
        })
      )

      set((state) => {
        const next = new Map(state.worktreeCache)
        next.set(projectPath, enriched)
        return { worktreeCache: next }
      })
    } catch {
      worktreeCacheTimestamps.delete(projectPath)
    }
  },

  sidebarProjectSort: (savedSidebar.projectSort as 'manual' | 'name' | 'recent') ?? 'manual',
  sidebarWorktreeSort: (savedSidebar.worktreeSort as 'name' | 'recent') ?? 'name',
  sidebarWorktreeFilter: (savedSidebar.worktreeFilter as 'all' | 'active') ?? 'all',
  sidebarViewMode: (savedSidebar.viewMode as SidebarViewMode) ?? 'worktrees-sessions',

  setSidebarProjectSort: (mode) => {
    saveSidebarSettings({ projectSort: mode })
    set({ sidebarProjectSort: mode })
  },
  setSidebarWorktreeSort: (mode) => {
    saveSidebarSettings({ worktreeSort: mode })
    set({ sidebarWorktreeSort: mode })
  },
  setSidebarWorktreeFilter: (filter) => {
    saveSidebarSettings({ worktreeFilter: filter })
    set({ sidebarWorktreeFilter: filter })
  },
  setSidebarViewMode: (mode) => {
    saveSidebarSettings({ viewMode: mode })
    set({ sidebarViewMode: mode })
  },

  reorderProjects: (fromIndex, toIndex) =>
    set((state) => {
      if (!state.config) return {}
      const activeWs = state.activeWorkspace
      const wsProjects = state.config.projects.filter(
        (p) => (p.workspaceId ?? 'personal') === activeWs
      )
      const reordered = [...wsProjects]
      const [moved] = reordered.splice(fromIndex, 1)
      reordered.splice(toIndex, 0, moved)
      let wsIdx = 0
      const projects = state.config.projects.map((p) => {
        if ((p.workspaceId ?? 'personal') === activeWs) return reordered[wsIdx++]
        return p
      })
      const updated = { ...state.config, projects }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  sidebarWorkflowFilter: (() => {
    const v = savedSidebar.workflowFilter
    return v === 'all' || v === 'manual' || v === 'scheduled' ? v : 'all'
  })(),
  setSidebarWorkflowFilter: (filter) => {
    saveSidebarSettings({ workflowFilter: filter })
    set({ sidebarWorkflowFilter: filter })
  },

  reorderWorkflows: (fromIndex, toIndex) =>
    set((state) => {
      if (!state.config || fromIndex === toIndex) return {}
      const activeWs = state.activeWorkspace
      const wsWorkflows = (state.config.workflows ?? []).filter(
        (w) => (w.workspaceId ?? 'personal') === activeWs
      )
      if (
        fromIndex < 0 ||
        fromIndex >= wsWorkflows.length ||
        toIndex < 0 ||
        toIndex >= wsWorkflows.length
      ) {
        return {}
      }
      const reordered = [...wsWorkflows]
      const [moved] = reordered.splice(fromIndex, 1)
      reordered.splice(toIndex, 0, moved)
      let wsIdx = 0
      const workflows = (state.config.workflows ?? []).map((w) => {
        if ((w.workspaceId ?? 'personal') === activeWs) return reordered[wsIdx++]
        return w
      })
      const updated = { ...state.config, workflows }
      window.api.saveConfig(updated)
      return { config: updated }
    })
})
