import { StateCreator } from 'zustand'
import { TerminalSession } from '../../shared/types'
import { AppStore, UISlice, SidebarViewMode, FlexibleLayoutRect, TaskSourceFilter } from './types'

const EMPTY_SESSIONS: TerminalSession[] = []
const WORKTREE_CACHE_TTL = 5_000
const worktreeCacheTimestamps = new Map<string, number>()
const GRID_STORAGE_KEY = 'vorn:gridSettings'
const SIDEBAR_STORAGE_KEY = 'vorn:sidebarSettings'
const FLEXIBLE_STORAGE_KEY = 'vorn:flexibleLayouts'

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
  pendingContextualWorkflowId: null,
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
  backgroundTrayCollapsed: false,
  toolbarMinimizedCollapsed: false,
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
  selectedTaskId: null,
  taskStatusFilter: 'all' as const,
  taskSourceFilter: 'all' as TaskSourceFilter,
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

  setPendingContextualWorkflowId: (id) => set({ pendingContextualWorkflowId: id }),

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
  setVisibleTerminalIds: (ids) => set({ visibleTerminalIds: ids }),
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
      if (next.has(id)) {
        next.delete(id)
      } else {
        next.add(id)
      }
      return { minimizedTerminals: next }
    }),

  toggleBackgroundTray: () =>
    set((state) => ({ backgroundTrayCollapsed: !state.backgroundTrayCollapsed })),
  toggleToolbarMinimizedCollapsed: () =>
    set((state) => ({ toolbarMinimizedCollapsed: !state.toolbarMinimizedCollapsed })),

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
  beginWorkflowsRunsLoad: () =>
    set((s) => ({ workflowsRunsInflight: s.workflowsRunsInflight + 1 })),
  endWorkflowsRunsLoad: () =>
    set((s) => ({ workflowsRunsInflight: Math.max(0, s.workflowsRunsInflight - 1) })),
  bumpWorkflowsRunsReload: () =>
    set((s) => ({ workflowsRunsReloadToken: s.workflowsRunsReloadToken + 1 })),
  setSelectedTaskId: (id) => set({ selectedTaskId: id }),
  setTaskStatusFilter: (filter) => set({ taskStatusFilter: filter }),
  setTaskSourceFilter: (filter) => set({ taskSourceFilter: filter }),
  setTaskDialogOpen: (open, defaultStatus) =>
    set({ isTaskDialogOpen: open, taskDialogDefaultStatus: defaultStatus ?? 'todo' }),
  setEditingTask: (task) => set({ editingTask: task }),

  setActiveTabId: (id) => set({ activeTabId: id }),

  workflowExecutions: new Map(),
  setWorkflowExecution: (id, execution) =>
    set((state) => {
      const next = new Map(state.workflowExecutions)
      next.set(id, execution)
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
