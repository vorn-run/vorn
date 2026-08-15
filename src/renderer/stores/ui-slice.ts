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
  DevicePaneState,
  TerminalsPaneState,
  CardSplit,
  isPromotedPane
} from './types'
import {
  filesPaneId,
  editorPaneId,
  browserPaneId,
  devicePaneId,
  terminalsPaneId,
  isTerminalPane,
  paneOwnerId,
  promotedCardSeq,
  promotedCardId
} from '../lib/pane-id'
import { normalizeUrl } from '../lib/browser-url'
import { confirmDiscard, confirmDiscardAll, clearDirty } from '../lib/editor-dirty'
import { clampSplitRatio, sanitizePaneWeights, DEVICE_SPLIT_RATIO } from '../lib/split-ratio'

const EMPTY_SESSIONS: TerminalSession[] = []
const WORKTREE_CACHE_TTL = 5_000
const worktreeCacheTimestamps = new Map<string, number>()
/**
 * Longer than the worktree TTL because the answer changes on the timescale of
 * adding a dependency, not of typing a command — and because a wrong answer
 * only costs one button, whereas re-probing costs a readdir per session row.
 */
const MOBILE_CACHE_TTL = 60_000
const mobileCacheTimestamps = new Map<string, number>()
const GRID_STORAGE_KEY = 'vorn:gridSettings'
const SIDEBAR_STORAGE_KEY = 'vorn:sidebarSettings'
const FLEXIBLE_STORAGE_KEY = 'vorn:flexibleLayouts'
const CARD_SPLITS_STORAGE_KEY = 'vorn:cardSplits'
const PANES_STORAGE_KEY = 'vorn:panes'
const TERMINAL_PANELS_STORAGE_KEY = 'vorn:terminalPanels'
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
 *
 * A popped-out card *is* a cell and holds a rect while it exists, but its rect
 * is not persisted across a restart either — see the note on the filter below
 * for why keeping one would hand a dead card's position to a live one.
 */
function loadFlexibleLayouts(): Record<string, FlexibleLayoutRect> {
  try {
    const raw = localStorage.getItem(FLEXIBLE_STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, FlexibleLayoutRect>
    // Sessions only. A card *is* a cell and holds a rect while it exists, but
    // its id is only unique within a run: the sequence behind it is seeded from
    // persisted panes, so a fresh launch restarts at zero and the next card to
    // be popped out of the same session takes the dead one's id — and, if these
    // were kept, its position and size.
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
      editors?: Record<string, string | { filePath: string; sessionId?: string }>
      browsers?: Record<
        string,
        string | { tabs?: string[]; activeTab?: number; sessionId?: string }
      >
    }
    const editorPanes = parsePersistedEditors(parsed.editors)
    const browserPanes = parsePersistedBrowsers(parsed.browsers)
    seedCardSeq(editorPanes.keys(), browserPanes.keys())
    return { filesPanes: new Set(parsed.files ?? []), editorPanes, browserPanes }
  } catch {
    return { filesPanes: new Set(), editorPanes: new Map(), browserPanes: new Map() }
  }
}

/**
 * Read the persisted editor panes.
 *
 * Older builds stored a bare path per session, before a pane could be keyed by
 * anything but its owner. Those read back with the key as the owner, which is
 * exactly what they meant — so upgrading neither loses an open file nor mistakes
 * one for a popped-out card.
 */
export function parsePersistedEditors(
  saved: Record<string, string | { filePath?: string; sessionId?: string }> | undefined
): Map<string, EditorPaneState> {
  const out = new Map<string, EditorPaneState>()
  for (const [id, entry] of Object.entries(saved ?? {})) {
    if (typeof entry === 'string') out.set(id, { filePath: entry, sessionId: id })
    else if (entry.filePath)
      out.set(id, { filePath: entry.filePath, sessionId: entry.sessionId ?? id })
  }
  return out
}

/**
 * Read the persisted browser panes.
 *
 * Older builds stored a single url string per session, before a pane could
 * hold tabs. Those are read as a one-tab pane rather than discarded, so
 * upgrading doesn't silently close the page someone had open. As with editors,
 * an entry with no recorded owner is owned by its key.
 */
export function parsePersistedBrowsers(
  saved:
    | Record<string, string | { tabs?: string[]; activeTab?: number; sessionId?: string }>
    | undefined
): Map<string, BrowserPaneState> {
  return new Map(
    Object.entries(saved ?? {}).map(([id, entry]) => {
      if (typeof entry === 'string') return [id, { tabs: [entry], activeTab: 0, sessionId: id }]
      const tabs = entry.tabs?.length ? entry.tabs : [DEFAULT_BROWSER_URL]
      const activeTab = Math.min(Math.max(entry.activeTab ?? 0, 0), tabs.length - 1)
      return [id, { tabs, activeTab, sessionId: entry.sessionId ?? id }]
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
        editors: Object.fromEntries(
          [...editorPanes].map(([id, s]) => [id, { filePath: s.filePath, sessionId: s.sessionId }])
        ),
        browsers: Object.fromEntries(
          [...browserPanes].map(([id, s]) => [
            id,
            { tabs: s.tabs, activeTab: s.activeTab, sessionId: s.sessionId }
          ])
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
  browserMemory: Map<string, BrowserPaneState>,
  devicePanes: Map<string, DevicePaneState>,
  terminalsPanes: Map<string, TerminalsPaneState>,
  cardSplits: Record<string, CardSplit>,
  liveSessionIds: Set<string>
): {
  filesPanes: Set<string>
  editorPanes: Map<string, EditorPaneState>
  browserPanes: Map<string, BrowserPaneState>
  browserMemory: Map<string, BrowserPaneState>
  devicePanes: Map<string, DevicePaneState>
  terminalsPanes: Map<string, TerminalsPaneState>
  cardSplits: Record<string, CardSplit>
} | null {
  const nextFiles = new Set([...filesPanes].filter((id) => liveSessionIds.has(id)))
  // On the record's owner, not on the key: a popped-out file or tab is keyed by
  // card id, and pruning by key would delete every one of them on the first
  // reconcile — silently discarding the pages and files someone put there.
  const nextEditors = new Map([...editorPanes].filter(([, e]) => liveSessionIds.has(e.sessionId)))
  const nextBrowsers = new Map([...browserPanes].filter(([, b]) => liveSessionIds.has(b.sessionId)))
  // Tabs remembered for a closed pane die with their session as well. This is
  // the path `removeTerminal` cannot cover: a session that simply never came
  // back leaves no removal to hang the cleanup off.
  const nextMemory = new Map([...browserMemory].filter(([id]) => liveSessionIds.has(id)))
  // Device panes are in-memory only, but a dead session's pane still keeps a
  // card mounted against a device nobody can drive — the same leak, minus the
  // localStorage growth.
  const nextDevices = new Map([...devicePanes].filter(([id]) => liveSessionIds.has(id)))
  // A panel goes with its owner, and its remaining shells go with it — they are
  // sessions too, so a dead session's list would otherwise keep ids that hide
  // terminals which no longer exist.
  //
  // The shells are checked one by one for the same reason: a shell that never
  // came back leaves its id in a panel whose owner did, and the tab would
  // return after a restart named after the raw id with nothing behind it.
  const nextPanels = new Map<string, TerminalsPaneState>()
  let panelsChanged = false
  for (const [id, pane] of terminalsPanes) {
    if (!liveSessionIds.has(id)) {
      panelsChanged = true
      continue
    }
    const terminals = pane.terminals.filter((tid) => liveSessionIds.has(tid))
    if (terminals.length === pane.terminals.length) {
      nextPanels.set(id, pane)
      continue
    }
    panelsChanged = true
    if (terminals.length === 0) continue
    nextPanels.set(id, { terminals, activeTab: Math.min(pane.activeTab, terminals.length - 1) })
  }
  const nextSplits = Object.fromEntries(
    Object.entries(cardSplits).filter(([id]) => liveSessionIds.has(id))
  )
  const splitsChanged = Object.keys(nextSplits).length !== Object.keys(cardSplits).length
  if (
    nextFiles.size === filesPanes.size &&
    nextEditors.size === editorPanes.size &&
    nextBrowsers.size === browserPanes.size &&
    nextMemory.size === browserMemory.size &&
    nextDevices.size === devicePanes.size &&
    !panelsChanged &&
    !splitsChanged
  ) {
    return null
  }
  savePanes(nextFiles, nextEditors, nextBrowsers)
  if (splitsChanged) saveCardSplits(nextSplits)
  if (panelsChanged) saveTerminalPanels(nextPanels)
  return {
    filesPanes: nextFiles,
    editorPanes: nextEditors,
    browserPanes: nextBrowsers,
    browserMemory: nextMemory,
    devicePanes: nextDevices,
    terminalsPanes: nextPanels,
    cardSplits: nextSplits
  }
}

/**
 * Let go of a terminal that a panel is holding.
 *
 * Extraction and closing both arrive here: one keeps the terminal alive and one
 * kills it, but either way the panel has stopped claiming the id. A claim left
 * behind outlives its terminal — the tab stays, falls back to naming itself
 * after the raw id, and draws a pane for a session the store no longer has.
 *
 * Emptied panels are reported rather than quietly dropped, because a panel's
 * pane id can be focused or maximized and that has to be released with it.
 */
export function releaseFromPanels(
  panes: Map<string, TerminalsPaneState>,
  terminalId: string
): { panes: Map<string, TerminalsPaneState>; emptied: string[] } | null {
  let next: Map<string, TerminalsPaneState> | null = null
  const emptied: string[] = []
  for (const [sessionId, pane] of panes) {
    const index = pane.terminals.indexOf(terminalId)
    if (index === -1) continue
    next ??= new Map(panes)
    const terminals = pane.terminals.filter((id) => id !== terminalId)
    if (terminals.length === 0) {
      // A panel with no shells is a box taking up a pane, the same rule the
      // browser's last tab follows.
      next.delete(sessionId)
      emptied.push(sessionId)
      continue
    }
    // Land on the neighbour rather than jumping to the far end, and shift when
    // something ahead of the active one leaves.
    next.set(sessionId, {
      terminals,
      activeTab: Math.max(
        0,
        Math.min(
          index <= pane.activeTab ? pane.activeTab - 1 : pane.activeTab,
          terminals.length - 1
        )
      )
    })
  }
  return next ? { panes: next, emptied } : null
}

/**
 * Drop every placement a pane held. For closing one.
 *
 * Placement lives apart from the pane records — four app-level fields keyed by
 * pane id — which is what lets ordering, maximize, minimize and focus address
 * any pane without knowing its kind. The cost is that closing a pane has to say
 * so explicitly, and each field strands differently when it doesn't:
 *
 * - `minimizedTerminals` leaves a dock entry that restores nothing.
 * - `maximizedPaneId` blanks its owner's card in favour of nothing.
 * - `focusedTerminalId` / `previewTerminalId` are the dangerous pair. The focus
 *   stage is chosen by "is anything focused", and the app drops its titlebar
 *   while something is — so an id pointing at a pane that no longer exists
 *   renders an empty window with no chrome and no way back except Escape.
 * - `selectedTerminalId` reaches the same place by a longer road: Cmd+O focuses
 *   the selection. A view does sweep a stale one, but only while a view that
 *   derives the visible list is mounted — which is not true on the focus stage,
 *   so the store cannot rely on being tidied up after.
 *
 * Every field is only written when it actually changes, so a close that touches
 * nothing returns nothing.
 */
function clearPlacement(
  state: Pick<
    AppStore,
    | 'maximizedPaneId'
    | 'minimizedTerminals'
    | 'focusedTerminalId'
    | 'previewTerminalId'
    | 'selectedTerminalId'
  >,
  paneId: string
): Partial<AppStore> {
  const cleared: Partial<AppStore> = {}
  if (state.maximizedPaneId === paneId) cleared.maximizedPaneId = null
  if (state.focusedTerminalId === paneId) cleared.focusedTerminalId = null
  if (state.previewTerminalId === paneId) cleared.previewTerminalId = null
  if (state.selectedTerminalId === paneId) cleared.selectedTerminalId = null
  if (state.minimizedTerminals.has(paneId)) {
    const minimized = new Set(state.minimizedTerminals)
    minimized.delete(paneId)
    cleared.minimizedTerminals = minimized
  }
  return cleared
}

/**
 * Sequence behind `card:` ids.
 *
 * Seeded past whatever the persisted panes already use, because promoted cards
 * survive a reload: starting from zero would reissue an id a restored card
 * still holds, and the second card would silently overwrite the first.
 *
 * Monotonic, never reused. A gap left by a closed card costs nothing, whereas
 * reusing its id would hand a new card that card's minimized state and grid rect.
 */
let cardSeq = 0

function seedCardSeq(...keys: Iterable<string>[]): void {
  for (const group of keys) {
    for (const key of group) {
      const seq = promotedCardSeq(key)
      if (seq !== null && seq >= cardSeq) cardSeq = seq + 1
    }
  }
}

function nextCardId(sessionId: string): string {
  return promotedCardId(sessionId, cardSeq++)
}

/** Whether two id lists say the same thing, in the same order. */
function sameIds(a: readonly string[], b: readonly string[]): boolean {
  if (a === b) return true
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
  return true
}

/**
 * Which shells each session holds, persisted so a reload keeps the arrangement.
 *
 * Shape: `{ [sessionId]: { terminals: sessionId[], activeTab: number } }`. An
 * entry whose terminals are all gone is dropped on read: a panel listing shells
 * that no longer exist would render an empty box, and its ids are how those
 * shells stay hidden from every other surface.
 */
export function parsePersistedPanels(
  parsed: Record<string, Partial<TerminalsPaneState>> | undefined
): Map<string, TerminalsPaneState> {
  const out = new Map<string, TerminalsPaneState>()
  for (const [id, pane] of Object.entries(parsed ?? {})) {
    const terminals = pane?.terminals?.filter((t) => typeof t === 'string') ?? []
    if (terminals.length === 0) continue
    // `?? 0` only catches null and undefined: a corrupted entry with a string
    // or a NaN here propagates through Math.min as NaN, and the card indexes
    // its tab list with it and dereferences undefined.
    const saved = Number.isInteger(pane?.activeTab) ? (pane?.activeTab as number) : 0
    const activeTab = Math.min(Math.max(saved, 0), terminals.length - 1)
    out.set(id, { terminals, activeTab })
  }
  return out
}

function loadTerminalPanels(): Map<string, TerminalsPaneState> {
  try {
    const raw = localStorage.getItem(TERMINAL_PANELS_STORAGE_KEY)
    return raw ? parsePersistedPanels(JSON.parse(raw)) : new Map()
  } catch {
    return new Map()
  }
}

export function saveTerminalPanels(panels: Map<string, TerminalsPaneState>): void {
  try {
    localStorage.setItem(TERMINAL_PANELS_STORAGE_KEY, JSON.stringify(Object.fromEntries(panels)))
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
  // Not persisted: remembering tabs across a close is about the current
  // sitting, and a reload already restores whatever was open from loadPanes.
  browserMemory: new Map(),
  // Not part of loadPanes: a claim lives in main and dies with the app, so a
  // device pane restored from disk would frame a simulator nobody holds.
  devicePanes: new Map(),
  terminalsPanes: loadTerminalPanels(),
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
      const unchanged = sameIds(state.visibleTerminalIds, ids)
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
              state.browserMemory,
              state.devicePanes,
              state.terminalsPanes,
              state.cardSplits,
              live
            )
          : null
      // An unchanged list is not a change worth notifying the app about — but
      // the reconcile above still has to run, because this is its only trigger.
      // Gating both on the list changing meant a launch where the visible set
      // never moved (everything filtered out, or every restored session
      // minimized) never pruned a dead session's panes at all.
      if (unchanged && !reconciled) return {}
      return { ...(unchanged ? {} : { visibleTerminalIds: ids }), ...(reconciled ?? {}) }
    }),
  setFocusableTerminalIds: (ids) =>
    set((state) => (sameIds(state.focusableTerminalIds, ids) ? {} : { focusableTerminalIds: ids })),

  reorderTerminals: (draggedId, droppedOnId) =>
    set((state) => {
      // Ids, not indices. The lists the grid and the tab strip drag within
      // interleave popped-out cards, while `terminalOrder` holds sessions only,
      // so an index from either was off by the number of cards ahead of it —
      // moving the wrong session, and when the index ran past the end, splicing
      // nothing and writing `undefined` into the order that is then persisted
      // and shipped to the server.
      //
      // A card is not in this list and has no position of its own: it is drawn
      // beside the session it came from. Dragging one is therefore a no-op, and
      // dropping onto one means the slot its owner occupies.
      const order = [...state.terminalOrder]
      const from = order.indexOf(draggedId)
      const to = order.indexOf(paneOwnerId(droppedOnId))
      if (from === -1 || to === -1 || from === to) return {}
      const [moved] = order.splice(from, 1)
      order.splice(to, 0, moved)
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
        ...clearPlacement(state, filesPaneId(sessionId))
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
      next.set(sessionId, { filePath, sessionId })
      savePanes(state.filesPanes, next, state.browserPanes)
      return { editorPanes: next }
    }),

  closeEditorPane: (paneId) =>
    set((state) => {
      const closing = state.editorPanes.get(paneId)
      if (!closing) return {}
      const next = new Map(state.editorPanes)
      next.delete(paneId)
      savePanes(state.filesPanes, next, state.browserPanes)
      return {
        editorPanes: next,
        ...clearPlacement(state, isPromotedPane(paneId, closing) ? paneId : editorPaneId(paneId))
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
          next.set(sessionId, { tabs: [normalized], activeTab: 0, sessionId })
        }
      } else if (!existing) {
        // Reopening picks up where the pane left off. Closing it is how you get
        // the space back, not how you throw the tabs away.
        const remembered = state.browserMemory.get(sessionId)
        next.set(sessionId, remembered ?? { tabs: [DEFAULT_BROWSER_URL], activeTab: 0, sessionId })
      } else {
        return {}
      }

      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  closeBrowserPane: (paneId) =>
    set((state) => {
      const closing = state.browserPanes.get(paneId)
      if (!closing) return {}
      const next = new Map(state.browserPanes)
      next.delete(paneId)
      savePanes(state.filesPanes, state.editorPanes, next)
      // Remembering the tabs is for reopening a session's own browser. A
      // popped-out tab has no reopen — closing its card is a discard, and
      // filing it here would resurface the page on the session's next open.
      const memory = new Map(state.browserMemory)
      if (!isPromotedPane(paneId, closing)) memory.set(paneId, closing)
      return {
        browserPanes: next,
        ...(isPromotedPane(paneId, closing) ? {} : { browserMemory: memory }),
        ...clearPlacement(state, isPromotedPane(paneId, closing) ? paneId : browserPaneId(paneId))
      }
    }),

  toggleBrowserPane: (sessionId) => {
    const { browserPanes, openBrowserPane, closeBrowserPane } = get()
    if (browserPanes.has(sessionId)) closeBrowserPane(sessionId)
    else openBrowserPane(sessionId)
  },

  addBrowserTab: (paneId, url) =>
    set((state) => {
      const existing = state.browserPanes.get(paneId)
      if (!existing) return {}
      const normalized = url === undefined ? DEFAULT_BROWSER_URL : normalizeUrl(url)
      if (!normalized) return {}
      const next = new Map(state.browserPanes)
      const tabs = [...existing.tabs, normalized]
      next.set(paneId, { ...existing, tabs, activeTab: tabs.length - 1 })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  closeBrowserTab: (paneId, index) => {
    const existing = get().browserPanes.get(paneId)
    if (!existing || index < 0 || index >= existing.tabs.length) return
    // The last tab going means the pane itself goes; an empty browser is just
    // a box taking up a grid cell. Closing a tab is a discard, though, not a
    // "give me the space back" — so unlike a pane close it leaves nothing to
    // restore, or reopening would hand back the page just thrown away.
    if (existing.tabs.length === 1) {
      get().closeBrowserPane(paneId)
      set((state) => {
        if (!state.browserMemory.has(paneId)) return {}
        const memory = new Map(state.browserMemory)
        memory.delete(paneId)
        return { browserMemory: memory }
      })
      return
    }
    set((state) => {
      const pane = state.browserPanes.get(paneId)
      if (!pane) return {}
      const tabs = pane.tabs.filter((_, i) => i !== index)
      // Closing a tab left of the active one shifts it; closing the active one
      // lands on its neighbour rather than jumping to the far end.
      const activeTab = Math.min(
        index <= pane.activeTab ? pane.activeTab - 1 : pane.activeTab,
        tabs.length - 1
      )
      const next = new Map(state.browserPanes)
      next.set(paneId, { ...pane, tabs, activeTab: Math.max(activeTab, 0) })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    })
  },

  setActiveBrowserTab: (paneId, index) =>
    set((state) => {
      const existing = state.browserPanes.get(paneId)
      if (!existing || index < 0 || index >= existing.tabs.length) return {}
      if (existing.activeTab === index) return {}
      const next = new Map(state.browserPanes)
      next.set(paneId, { ...existing, activeTab: index })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    }),

  openDevicePane: (sessionId, device) =>
    set((state) => {
      const existing = state.devicePanes.get(sessionId)
      if (existing && existing.udid === device.udid && existing.name === device.name) return {}
      const next = new Map(state.devicePanes)
      next.set(sessionId, device)
      // A phone is roughly 0.46 as wide as it is tall, so the even split every
      // other pane kind wants renders it as a narrow strip of screen floating in
      // a wide field of empty background — while the terminal, which needs the
      // width, is squeezed to half a card. Bias the card toward the terminal the
      // first time a device arrives. Only when the person has not already sized
      // this card themselves: their ratio is a decision, not a default.
      if (state.cardSplits[sessionId]) return { devicePanes: next }
      const splits = {
        ...state.cardSplits,
        [sessionId]: { terminal: DEVICE_SPLIT_RATIO, panes: [] }
      }
      saveCardSplits(splits)
      return { devicePanes: next, cardSplits: splits }
    }),

  claimAndOpenDevicePane: async (sessionId, device) => {
    try {
      // Claiming first is what makes the pane's first poll succeed. It also
      // boots the simulator if it is not running, so the person never has to
      // leave Vorn for Xcode.
      const claimed = await window.api.deviceClaim(sessionId, device.udid)
      get().openDevicePane(sessionId, { udid: claimed.udid, name: claimed.name })
      return null
    } catch (e) {
      // Surfaced rather than swallowed: the likeliest failure is another
      // session holding the device, and that message names the holder.
      return e instanceof Error ? e.message : String(e)
    }
  },

  closeDevicePane: (sessionId) =>
    set((state) => {
      if (!state.devicePanes.has(sessionId)) return {}
      // Closing the pane hands the device back. Holding a claim for a pane
      // nobody is looking at locks the simulator out of every other session
      // with nothing on screen to explain why. Fire-and-forget: a failed
      // release must not block the pane from closing, and main releases on
      // session teardown regardless.
      try {
        void window.api.deviceRelease?.(sessionId)?.catch(() => {})
      } catch {
        // A release that throws synchronously must still not trap the pane
        // open — main releases on session teardown regardless.
      }
      const next = new Map(state.devicePanes)
      next.delete(sessionId)
      return {
        devicePanes: next,
        ...clearPlacement(state, devicePaneId(sessionId))
      }
    }),

  openTerminalsPane: (sessionId, terminalId) =>
    set((state) => {
      const next = new Map(state.terminalsPanes)
      const existing = next.get(sessionId)?.terminals ?? []
      // Claimed already means show it, not list it twice: one terminal in two
      // tabs is two slots fighting over a single rendered wrapper.
      const terminals = existing.includes(terminalId) ? existing : [...existing, terminalId]
      next.set(sessionId, { terminals, activeTab: terminals.indexOf(terminalId) })
      saveTerminalPanels(next)
      return { terminalsPanes: next }
    }),

  closeTerminalsPane: (sessionId) =>
    set((state) => {
      if (!state.terminalsPanes.has(sessionId)) return {}
      const next = new Map(state.terminalsPanes)
      next.delete(sessionId)
      saveTerminalPanels(next)
      return {
        terminalsPanes: next,
        ...clearPlacement(state, terminalsPaneId(sessionId))
      }
    }),

  setActivePanelTerminal: (sessionId, index) =>
    set((state) => {
      const pane = state.terminalsPanes.get(sessionId)
      if (!pane || index < 0 || index >= pane.terminals.length) return {}
      if (pane.activeTab === index) return {}
      const next = new Map(state.terminalsPanes)
      next.set(sessionId, { ...pane, activeTab: index })
      saveTerminalPanels(next)
      return { terminalsPanes: next }
    }),

  extractPanelTerminal: (sessionId, terminalId) =>
    set((state) => {
      // Extraction is nothing but releasing the claim — the terminal was a
      // session all along, and is now simply nobody's.
      const released = releaseFromPanels(state.terminalsPanes, terminalId)
      if (!released) return {}
      saveTerminalPanels(released.panes)
      // Keyed off what was emptied, not off the caller's sessionId: those can
      // disagree, and the panel that actually went would keep its focus and
      // maximize pointing at a pane that no longer draws.
      return {
        terminalsPanes: released.panes,
        ...released.emptied.reduce(
          (cleared, ownerId) => ({
            ...cleared,
            ...clearPlacement(state, terminalsPaneId(ownerId))
          }),
          {}
        )
      }
    }),

  setMaximizedPane: (paneId) =>
    set((state) => (state.maximizedPaneId === paneId ? {} : { maximizedPaneId: paneId })),

  promoteFile: (sessionId, filePath) => {
    // One card per file per session. Two cards on one path is two editors over
    // one file on disk, each with its own buffer and its own dirty flag: save in
    // one, then save in the other, and the second silently writes its stale copy
    // over the first. Nothing anywhere would report it.
    for (const [existingId, pane] of get().editorPanes) {
      if (isPromotedPane(existingId, pane) && pane.sessionId === sessionId) {
        if (pane.filePath !== filePath) continue
        // Surface the one that exists rather than returning an id the caller
        // then ignores: popping out an already-popped file that had been
        // minimized otherwise looked like the control did nothing at all.
        set((state) =>
          state.minimizedTerminals.has(existingId)
            ? {
                minimizedTerminals: new Set(
                  [...state.minimizedTerminals].filter((id) => id !== existingId)
                )
              }
            : {}
        )
        return existingId
      }
    }

    const cardId = nextCardId(sessionId)
    set((state) => {
      const next = new Map(state.editorPanes)
      next.set(cardId, { filePath, sessionId })
      savePanes(state.filesPanes, next, state.browserPanes)
      return { editorPanes: next }
    })
    return cardId
  },

  promoteBrowserTab: (paneId, index) => {
    const pane = get().browserPanes.get(paneId)
    if (!pane || index < 0 || index >= pane.tabs.length) return null
    const url = pane.tabs[index]
    const cardId = nextCardId(pane.sessionId)
    // Out of the strip before into the card: leaving it in both would mount two
    // guests on one url, each with its own scroll position and half-typed form,
    // and closing either would look like the page had refused to go away.
    get().closeBrowserTab(paneId, index)
    set((state) => {
      const next = new Map(state.browserPanes)
      next.set(cardId, { tabs: [url], activeTab: 0, sessionId: pane.sessionId })
      savePanes(state.filesPanes, state.editorPanes, next)
      return { browserPanes: next }
    })
    return cardId
  },

  returnCardToSession: (cardId) => {
    const state = get()
    const editor = state.editorPanes.get(cardId)
    const browser = state.browserPanes.get(cardId)

    if (editor && isPromotedPane(cardId, editor)) {
      // Into the session's editor, which holds one file — so this displaces
      // whatever was there, exactly as picking a file in the tree does, and has
      // to ask the same question before throwing that buffer away. The card's
      // own buffer goes too: it is a different editor under a different id.
      // Both buffers in one question. Asked separately, answering yes then no
      // cleared the session editor's dirty flag and then bailed — leaving those
      // edits on screen with nothing left to prompt about them ever again.
      if (!confirmDiscardAll([editor.sessionId, cardId])) return
      state.openEditorPane(editor.sessionId, editor.filePath)
      state.closeEditorPane(cardId)
      return
    }

    if (browser && isPromotedPane(cardId, browser)) {
      // Every tab, not just the active one. A card can gather tabs of its own —
      // its strip keeps its `+` — and returning it used to carry back whichever
      // page happened to be in front and drop the rest without a word.
      const [first, ...rest] = browser.tabs
      if (state.browserPanes.has(browser.sessionId)) state.addBrowserTab(browser.sessionId, first)
      // If that browser is closed, opening it on this page is the landing spot —
      // a tab with nowhere to go would leave the card the only thing holding it,
      // and it is about to close.
      else state.openBrowserPane(browser.sessionId, first)

      // If the landing failed — a url that no longer normalizes, from a
      // corrupted store — every add below would no-op and the close would take
      // the whole card with it. Keep the card instead; it is the only copy.
      const landed = get().browserPanes.get(browser.sessionId)
      if (!landed) return

      const firstIndex = landed.tabs.length - 1
      for (const url of rest) state.addBrowserTab(browser.sessionId, url)
      // `addBrowserTab` activates what it adds, so without this the strip ends
      // up showing the card's *last* page rather than the one being looked at.
      state.setActiveBrowserTab(browser.sessionId, firstIndex + browser.activeTab)
      state.closeBrowserPane(cardId)
    }
  },

  closeCard: (cardId) => {
    const state = get()
    const editor = state.editorPanes.get(cardId)
    if (editor && isPromotedPane(cardId, editor)) {
      // The confirm lives here rather than at each button, because it was
      // missing from two of the three: the card's own ✕ asked, while its tab
      // and its sidebar row threw the buffer away silently.
      if (!confirmDiscard(cardId)) return
      clearDirty(cardId)
      state.closeEditorPane(cardId)
      return
    }
    const browser = state.browserPanes.get(cardId)
    if (browser && isPromotedPane(cardId, browser)) state.closeBrowserPane(cardId)
  },

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

  // Seeded from main so a reload mid-download does not start from scratch.
  // `unsupported` is the honest starting point: in a dev build no updater
  // event will ever arrive.
  appUpdateStatus: { kind: 'unsupported' },
  setAppUpdateStatus: (status) =>
    set((state) => ({
      appUpdateStatus: status,
      // A newer version re-earns the banner: dismissal applied to the update
      // the user waved off, not to every update from here on.
      updateBannerDismissed:
        status.kind === 'ready' && state.appUpdateStatus.kind === 'ready'
          ? state.appUpdateStatus.version === status.version && state.updateBannerDismissed
          : status.kind === 'ready'
            ? false
            : state.updateBannerDismissed
    })),
  updateBannerDismissed: false,
  setUpdateBannerDismissed: (dismissed) => set({ updateBannerDismissed: dismissed }),

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

  mobileProjectCache: new Map(),
  loadMobileProject: async (projectPath, force) => {
    if (!projectPath) return
    if (!force) {
      const lastLoaded = mobileCacheTimestamps.get(projectPath)
      if (lastLoaded && Date.now() - lastLoaded < MOBILE_CACHE_TTL) return
    }
    // Stamped before the await, not after: several session rows for the same
    // project mount at once, and without this every one of them would fire its
    // own probe against the same directory.
    mobileCacheTimestamps.set(projectPath, Date.now())

    try {
      const result = await window.api.detectMobileProject(projectPath)
      set((state) => {
        const next = new Map(state.mobileProjectCache)
        next.set(projectPath, result)
        return { mobileProjectCache: next }
      })
    } catch {
      // Clearing the stamp is what makes a failure retry rather than pin the
      // project as unprobed for the whole TTL.
      mobileCacheTimestamps.delete(projectPath)
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

/**
 * Which of a session's child panes are open, and whether any is.
 *
 * Every site that decides whether to mount a session's pane column used to
 * spell this list out itself, and each new pane kind then had to be added to
 * all of them. The device pane shipped with two of the four updated, so it
 * landed in the store, reported success, and never rendered in either the card
 * grid or tab view — a pane that opens into nothing, with no error anywhere to
 * say why. One selector, so a new kind is added once.
 */
export function selectPaneFlags(
  s: Pick<
    AppStore,
    'filesPanes' | 'editorPanes' | 'browserPanes' | 'devicePanes' | 'terminalsPanes'
  >,
  sessionId: string | null
): {
  files: boolean
  editor: boolean
  browser: boolean
  device: boolean
  terminals: boolean
  any: boolean
} {
  const files = sessionId ? s.filesPanes.has(sessionId) : false
  const editor = sessionId ? s.editorPanes.has(sessionId) : false
  const browser = sessionId ? s.browserPanes.has(sessionId) : false
  const device = sessionId ? s.devicePanes.has(sessionId) : false
  const terminals = sessionId ? s.terminalsPanes.has(sessionId) : false
  return {
    files,
    editor,
    browser,
    device,
    terminals,
    any: files || editor || browser || device || terminals
  }
}
