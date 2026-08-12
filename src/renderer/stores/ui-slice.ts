import { StateCreator } from 'zustand'
import { TerminalSession } from '../../shared/types'
import {
  AppStore,
  UISlice,
  SidebarViewMode,
  FlexibleLayoutRect,
  TaskSourceFilter,
  EditorPaneState,
  BrowserPaneState,
  CardSplit
} from './types'
import { filesPaneId, editorPaneId, browserPaneId, isTerminalPane } from '../lib/pane-id'
import { normalizeUrl } from '../lib/browser-url'
import { clampSplitRatio, sanitizePaneWeights } from '../lib/split-ratio'

const EMPTY_SESSIONS: TerminalSession[] = []
const WORKTREE_CACHE_TTL = 5_000
const worktreeCacheTimestamps = new Map<string, number>()
const GRID_STORAGE_KEY = 'vorn:gridSettings'
const SIDEBAR_STORAGE_KEY = 'vorn:sidebarSettings'
const FLEXIBLE_STORAGE_KEY = 'vorn:flexibleLayouts'
const CARD_SPLITS_STORAGE_KEY = 'vorn:cardSplits'
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

/**
 * Saved card rects, keyed by each session's stable id.
 *
 * An older build gave every child pane its own grid cell and so its own rect,
 * under a `files:` / `editor:` / `browser:` prefixed key. Panes now live inside
 * their owner's card, leaving those keys unreachable — dropping them on read
 * keeps the store from carrying dead weight forward forever.
 */
function loadFlexibleLayouts(): Record<string, FlexibleLayoutRect> {
  try {
    const raw = localStorage.getItem(FLEXIBLE_STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, FlexibleLayoutRect>
    const live = Object.fromEntries(Object.entries(parsed).filter(([key]) => isTerminalPane(key)))
    if (Object.keys(live).length !== Object.keys(parsed).length) saveFlexibleLayouts(live)
    return live
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
 * How each session card divides its interior, keyed by session id.
 *
 * Ratios are clamped and non-finite values dropped on read: a corrupted entry
 * would otherwise render a card with a zero-width terminal, which no amount of
 * dragging can recover because the divider itself would be off-screen.
 */
function loadCardSplits(): Record<string, CardSplit> {
  try {
    const raw = localStorage.getItem(CARD_SPLITS_STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, Partial<CardSplit>>
    const out: Record<string, CardSplit> = {}
    for (const [id, split] of Object.entries(parsed)) {
      if (!split || typeof split !== 'object') continue
      out[id] = {
        terminal: clampSplitRatio(Number(split.terminal)),
        panes: sanitizePaneWeights(split.panes)
      }
    }
    return out
  } catch {
    return {}
  }
}

function saveCardSplits(splits: Record<string, CardSplit>): void {
  try {
    localStorage.setItem(CARD_SPLITS_STORAGE_KEY, JSON.stringify(splits))
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
 * Drop persisted pane and card-split entries whose session no longer exists.
 *
 * `removeTerminal` prunes sessions closed during a run, but a session that
 * simply never came back after a restart leaves its entry behind. Reconciling
 * against the live set keeps localStorage from growing without bound and stops
 * a recycled id from inheriting a stale pane or a mysteriously wrong divider
 * position.
 */
function reconcilePanes(
  filesPanes: Set<string>,
  editorPanes: Map<string, EditorPaneState>,
  browserPanes: Map<string, BrowserPaneState>,
  cardSplits: Record<string, CardSplit>,
  liveSessionIds: Set<string>
): {
  filesPanes: Set<string>
  editorPanes: Map<string, EditorPaneState>
  browserPanes: Map<string, BrowserPaneState>
  cardSplits: Record<string, CardSplit>
} | null {
  const nextFiles = new Set([...filesPanes].filter((id) => liveSessionIds.has(id)))
  const nextEditors = new Map([...editorPanes].filter(([id]) => liveSessionIds.has(id)))
  const nextBrowsers = new Map([...browserPanes].filter(([id]) => liveSessionIds.has(id)))
  const nextSplits = Object.fromEntries(
    Object.entries(cardSplits).filter(([id]) => liveSessionIds.has(id))
  )
  const splitsChanged = Object.keys(nextSplits).length !== Object.keys(cardSplits).length
  if (
    nextFiles.size === filesPanes.size &&
    nextEditors.size === editorPanes.size &&
    nextBrowsers.size === browserPanes.size &&
    !splitsChanged
  ) {
    return null
  }
  savePanes(nextFiles, nextEditors, nextBrowsers)
  if (splitsChanged) saveCardSplits(nextSplits)
  return {
    filesPanes: nextFiles,
    editorPanes: nextEditors,
    browserPanes: nextBrowsers,
    cardSplits: nextSplits
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
  cardSplits: loadCardSplits(),
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

  setCardSplit: (sessionId, split) =>
    set((state) => {
      const next = {
        ...state.cardSplits,
        [sessionId]: {
          terminal: clampSplitRatio(split.terminal),
          panes: sanitizePaneWeights(split.panes)
        }
      }
      saveCardSplits(next)
      return { cardSplits: next }
    }),

  setTerminalOrder: (order) => set({ terminalOrder: order }),
  setVisibleTerminalIds: (ids) =>
    set((state) => {
      // Sessions restore by their persisted id, so pane entries stay valid across
      // restarts — but a session that never comes back would leave its entry
      // behind forever. Reconcile against the live set as it settles.
      const live = new Set(state.terminals.keys())
      const reconciled =
        live.size > 0
          ? reconcilePanes(
              state.filesPanes,
              state.editorPanes,
              state.browserPanes,
              state.cardSplits,
              live
            )
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
      if (state.filesPanes.has(sessionId)) return {}
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
      return {
        filesPanes: next,
        ...(state.maximizedPaneId === filesPaneId(sessionId) ? { maximizedPaneId: null } : {})
      }
    }),

  toggleFilesPane: (sessionId) => {
    const { filesPanes, openFilesPane, closeFilesPane } = get()
    if (filesPanes.has(sessionId)) closeFilesPane(sessionId)
    else openFilesPane(sessionId)
  },

  openEditorPane: (sessionId, filePath) =>
    set((state) => {
      const next = new Map(state.editorPanes)
      next.set(sessionId, { filePath })
      savePanes(state.filesPanes, next, state.browserPanes)
      return { editorPanes: next }
    }),

  closeEditorPane: (sessionId) =>
    set((state) => {
      if (!state.editorPanes.has(sessionId)) return {}
      const next = new Map(state.editorPanes)
      next.delete(sessionId)
      savePanes(state.filesPanes, next, state.browserPanes)
      return {
        editorPanes: next,
        ...(state.maximizedPaneId === editorPaneId(sessionId) ? { maximizedPaneId: null } : {})
      }
    }),

  openBrowserPane: (sessionId, url) =>
    set((state) => {
      // No url means "show me this session's browser" — keep whatever page it
      // already had rather than resetting it to blank.
      const next = new Map(state.browserPanes)
      const existing = next.get(sessionId)
      if (url !== undefined) {
        const normalized = normalizeUrl(url)
        if (!normalized) return {}
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
      } else {
        return {}
      }

      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  closeBrowserPane: (sessionId) =>
    set((state) => {
      if (!state.browserPanes.has(sessionId)) return {}
      const next = new Map(state.browserPanes)
      next.delete(sessionId)
      savePanes(state.filesPanes, state.editorPanes, next)
      return {
        browserPanes: next,
        ...(state.maximizedPaneId === browserPaneId(sessionId) ? { maximizedPaneId: null } : {})
      }
    }),

  toggleBrowserPane: (sessionId) => {
    const { browserPanes, openBrowserPane, closeBrowserPane } = get()
    if (browserPanes.has(sessionId)) closeBrowserPane(sessionId)
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
