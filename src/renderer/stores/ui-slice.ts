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
  BrowserTabState,
  DevicePaneState,
  DeviceRestoreRefusal,
  TerminalsPaneState,
  CardSplit,
  isPromotedPane,
  tabUrl
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
import { pruneScrollAnchors } from '../lib/scroll-anchor'
import { pruneDrafts } from '../lib/editor-drafts'
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
const DEVICE_PANES_STORAGE_KEY = 'vorn:devicePanes'
const VIEW_STORAGE_KEY = 'vorn:view'
/**
 * Where a browser pane opened with no url starts. Deliberately blank: guessing
 * a page would be wrong more often than not, and the address bar is focused
 * and empty, which is the prompt to type one.
 */
const DEFAULT_BROWSER_URL = 'about:blank'

interface PersistedView {
  minimized: string[]
  activeTabId: string | null
  maximizedPaneId: string | null
  activeProject: string | null
  activeWorktreePath: string | null
}

const EMPTY_VIEW: PersistedView = {
  minimized: [],
  activeTabId: null,
  maximizedPaneId: null,
  activeProject: null,
  activeWorktreePath: null
}

// Pane ids, the namespace focus and minimise already use -- not session ids.
function loadView(): PersistedView {
  try {
    const raw = localStorage.getItem(VIEW_STORAGE_KEY)
    if (!raw) return EMPTY_VIEW
    const parsed = JSON.parse(raw) as Partial<PersistedView>
    return {
      minimized: Array.isArray(parsed.minimized) ? parsed.minimized.filter(isNonEmpty) : [],
      activeTabId: orNull(parsed.activeTabId),
      maximizedPaneId: orNull(parsed.maximizedPaneId),
      activeProject: orNull(parsed.activeProject),
      activeWorktreePath: orNull(parsed.activeWorktreePath)
    }
  } catch {
    return EMPTY_VIEW
  }
}

function isNonEmpty(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0
}

function orNull(value: unknown): string | null {
  return isNonEmpty(value) ? value : null
}

/** The reader, for tests: everything else reads it once at construction. */
export const loadViewForTest = loadView

export { loadView, saveView }

function saveView(view: Partial<PersistedView>): void {
  try {
    const merged = { ...loadView(), ...view }
    localStorage.setItem(VIEW_STORAGE_KEY, JSON.stringify(merged))
  } catch {
    /* ignore */
  }
}

/**
 * The devices that were open when the app last closed, keyed by session.
 *
 * A request rather than a pane. Every other pane kind can be put straight back
 * because nothing outside this window has to agree; a device has to be taken
 * from the machine, and the claim it was taken with died with the last process.
 * So this is read by `restoreDevicePanes`, which asks for each one again, and
 * never by the slice's constructor.
 */
function loadDeviceRequests(): Map<string, DevicePaneState> {
  try {
    const raw = localStorage.getItem(DEVICE_PANES_STORAGE_KEY)
    if (!raw) return new Map()
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return new Map()
    const out = new Map<string, DevicePaneState>()
    for (const [sessionId, value] of Object.entries(parsed as Record<string, unknown>)) {
      const device = value as Partial<DevicePaneState>
      if (!isNonEmpty(device?.udid) || !isNonEmpty(device?.name)) continue
      out.set(sessionId, { udid: device.udid, name: device.name })
    }
    return out
  } catch {
    return new Map()
  }
}

/**
 * Take back the split `openDevicePane` biased toward the terminal.
 *
 * That ratio is written to storage, and nothing used to remove it -- so closing
 * the pane, or quitting with one open, left a card permanently sized for a phone
 * that is not there. Only when it is still the ratio we wrote: a person who has
 * dragged the divider since has made a decision, and this is not the place to
 * overrule it.
 */
function dropDeviceSplit(
  state: Pick<AppStore, 'cardSplits'>,
  sessionId: string
): { cardSplits?: Record<string, CardSplit> } {
  const split = state.cardSplits[sessionId]
  if (!split || split.terminal !== DEVICE_SPLIT_RATIO || split.panes.length > 0) return {}
  const splits = { ...state.cardSplits }
  delete splits[sessionId]
  saveCardSplits(splits)
  return { cardSplits: splits }
}

/**
 * Write one device down, leaving the rest of the record alone.
 *
 * Never the whole in-memory map. On a launch that map starts empty and fills a
 * device at a time as each claim comes back, so saving it wholesale made the
 * first success erase every request still waiting its turn -- including the ones
 * that were about to be refused and kept for the next launch. The stored record
 * legitimately holds entries this window has not got to yet.
 */
function rememberDeviceRequest(sessionId: string, device: DevicePaneState): void {
  const next = loadDeviceRequests()
  const held = next.get(sessionId)
  if (held?.udid === device.udid && held.name === device.name) return
  next.set(sessionId, device)
  saveDeviceRequests(next)
}

function forgetDeviceRequest(sessionId: string): void {
  const next = loadDeviceRequests()
  if (!next.delete(sessionId)) return
  saveDeviceRequests(next)
}

/** Drop requests for sessions that are gone, alongside the rest of the view state. */
function pruneDeviceRequests(liveSessionIds: Set<string>): void {
  const next = loadDeviceRequests()
  let changed = false
  for (const sessionId of [...next.keys()]) {
    if (liveSessionIds.has(sessionId)) continue
    next.delete(sessionId)
    changed = true
  }
  if (changed) saveDeviceRequests(next)
}

function saveDeviceRequests(devicePanes: Map<string, DevicePaneState>): void {
  try {
    localStorage.setItem(DEVICE_PANES_STORAGE_KEY, JSON.stringify(Object.fromEntries(devicePanes)))
  } catch {
    /* ignore */
  }
}

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
 * Two older shapes have to keep working. The oldest stored a single url string
 * per session, before a pane could hold tabs; the next stored `tabs` as a bare
 * string array, before a tab carried what the guest reported about itself.
 * Both are read rather than discarded, so upgrading doesn't silently close the
 * page someone had open. As with editors, an entry with no recorded owner is
 * owned by its key.
 *
 * A persisted tab restores as intent only: `liveUrl` and `title` are things a
 * live guest reports, and reviving them from disk would state as observed fact
 * something no guest has said this run.
 */
export function parsePersistedBrowsers(
  saved:
    | Record<
        string,
        | string
        | {
            tabs?: (string | { url?: string })[]
            activeTab?: number
            sessionId?: string
          }
      >
    | undefined
): Map<string, BrowserPaneState> {
  return new Map(
    Object.entries(saved ?? {}).map(([id, entry]) => {
      if (typeof entry === 'string') {
        return [id, { tabs: [{ url: entry }], activeTab: 0, sessionId: id }]
      }
      const urls = (entry.tabs ?? [])
        .map((t) => (typeof t === 'string' ? t : t?.url))
        .filter((u): u is string => typeof u === 'string' && u.length > 0)
      const tabs: BrowserTabState[] = (urls.length ? urls : [DEFAULT_BROWSER_URL]).map((url) => ({
        url
      }))
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
            {
              // Intent only. `liveUrl` and `title` are what a live guest
              // reported; writing them to disk would have the next run assert
              // as observed fact something no guest has said yet.
              tabs: s.tabs.map((t) => ({ url: t.url })),
              activeTab: s.activeTab,
              sessionId: s.sessionId
            }
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
  // Keyed by pane, so pruned against the panes that survived rather than against
  // the sessions -- an editor popped out into a card outlives its owner's pane
  // and its draft has to outlive it too.
  pruneDrafts(new Set(nextEditors.keys()))
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
 * Drop view state pointing at sessions that never came back.
 *
 * The same reasoning as `reconcilePanes`, on the fields that key by pane id
 * rather than by session: a card minimised before a quit whose session is gone
 * would stay in the pill row forever, naming a session nobody can restore.
 *
 * `paneOwnerId` is what makes one live set answer for all three — a terminal
 * pane is its own owner, so a raw session id passes through unchanged and a
 * `browser:`/`card:` id resolves to the session it hangs off.
 *
 * The project and worktree are deliberately not touched: they name a checkout
 * on disk, not a session, and a workspace with nothing running in it is still
 * the one you were last looking at.
 */
function reconcileView(
  minimized: Set<string>,
  activeTabId: string | null,
  maximizedPaneId: string | null,
  liveSessionIds: Set<string>
): {
  minimizedTerminals: Set<string>
  activeTabId: string | null
  maximizedPaneId: string | null
} | null {
  // Keyed by session id and pruned here rather than on its own trigger: a
  // scroll position is view state, and it goes when the rest of it goes.
  pruneScrollAnchors(liveSessionIds)
  pruneDeviceRequests(liveSessionIds)
  const alive = (paneId: string): boolean => liveSessionIds.has(paneOwnerId(paneId))
  const nextMinimized = new Set([...minimized].filter(alive))
  const nextTab = activeTabId && alive(activeTabId) ? activeTabId : null
  const nextMax = maximizedPaneId && alive(maximizedPaneId) ? maximizedPaneId : null
  if (
    nextMinimized.size === minimized.size &&
    nextTab === activeTabId &&
    nextMax === maximizedPaneId
  ) {
    return null
  }
  saveView({
    minimized: [...nextMinimized],
    activeTabId: nextTab,
    maximizedPaneId: nextMax
  })
  return { minimizedTerminals: nextMinimized, activeTabId: nextTab, maximizedPaneId: nextMax }
}

/**
 * Drop everything belonging to a session the server does not have.
 *
 * Sessions restore by their persisted id, so pane entries stay valid across
 * restarts — but a session that never comes back would leave its entry behind
 * forever.
 *
 * Reconciled against what the server has rather than against what is on the
 * board. Those differ on the launch where reopen is off: the ended sessions are
 * deliberately left off the board and offered by the banner instead, and pruning
 * against the board would delete the panes of every one of them a moment before
 * the person is asked whether to bring them back. Null until a sync pass has
 * asked, which is not the same as none.
 *
 * Returns null when there is nothing to change, so a caller can tell a reconcile
 * that did something from one that did not.
 */
function pruneAgainstKnown(state: AppStore): Partial<AppStore> | null {
  const live = state.knownSessionIds
  if (live === null) return null
  const panes = reconcilePanes(
    state.filesPanes,
    state.editorPanes,
    state.browserPanes,
    state.browserMemory,
    state.devicePanes,
    state.terminalsPanes,
    state.cardSplits,
    live
  )
  const view = reconcileView(
    state.minimizedTerminals,
    state.activeTabId,
    state.maximizedPaneId,
    live
  )
  if (!panes && !view) return null
  return { ...(panes ?? {}), ...(view ?? {}) }
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
  importedRequirements: null,
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
  knownSessionIds: null,
  focusableTerminalIds: [],
  minimizedTerminals: new Set(loadView().minimized),
  ...loadPanes(),
  // Not persisted: remembering tabs across a close is about the current
  // sitting, and a reload already restores whatever was open from loadPanes.
  browserMemory: new Map(),
  // Not part of loadPanes: a claim lives in main and dies with the app, so a
  // device pane restored from disk would frame a simulator nobody holds.
  devicePanes: new Map(),
  terminalsPanes: loadTerminalPanels(),
  maximizedPaneId: loadView().maximizedPaneId,
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
  activeTabId: loadView().activeTabId,

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

  // Closing forgets what an import left unbound: it was about the workflow being left.
  setWorkflowEditorOpen: (open) =>
    set(
      open
        ? { isWorkflowEditorOpen: true }
        : { isWorkflowEditorOpen: false, importedRequirements: null }
    ),

  setPendingWorkflowRun: (workflowId, context, targetNodeId) =>
    set({ pendingWorkflowRun: workflowId ? { workflowId, context, targetNodeId } : null }),

  setEditingWorkflowId: (id) => set({ editingWorkflowId: id }),

  setImportedRequirements: (value) => set({ importedRequirements: value }),

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
      const pruned = pruneAgainstKnown(state)
      if (unchanged && !pruned) return {}
      return { ...(unchanged ? {} : { visibleTerminalIds: ids }), ...(pruned ?? {}) }
    }),

  // Learning what exists is itself a reason to reconcile, and for a while the
  // only trigger was the visible list changing. A launch that finds nothing
  // running changes no list -- so a board whose every session was gone was the
  // one board that never pruned, and its panes and pills stayed for good.
  setKnownSessions: (ids) =>
    set((state) => {
      const known = new Set(ids)
      return {
        knownSessionIds: known,
        ...(pruneAgainstKnown({ ...state, knownSessionIds: known }) ?? {})
      }
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
      saveView({ minimized: [...next], ...(clearMax ? { maximizedPaneId: null } : {}) })
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

  openBrowserPane: (sessionId, url, opts) =>
    set((state) => {
      // No url means "show me this session's browser" — keep whatever page it
      // already had rather than resetting it to blank.
      const next = new Map(state.browserPanes)
      const existing = next.get(sessionId)
      if (url !== undefined) {
        // A url main vetted is taken as given: it already decided the scheme is
        // allowed and, for `file:`, that the path is inside the session's root.
        // Re-normalizing here has no filesystem to ask, so it would refuse
        // every `file:` url main just approved.
        const normalized = opts?.trusted ? url : normalizeUrl(url)
        if (!normalized) return {}
        if (existing) {
          // Navigating replaces the page in the tab the user is looking at,
          // the way an address bar does — it does not spawn a tab. New intent
          // drops the old observation with it: what the previous page reported
          // about itself says nothing about the one now loading.
          const tabs = [...existing.tabs]
          tabs[existing.activeTab] = { url: normalized }
          next.set(sessionId, { ...existing, tabs })
        } else {
          next.set(sessionId, { tabs: [{ url: normalized }], activeTab: 0, sessionId })
        }
      } else if (!existing) {
        // Reopening picks up where the pane left off. Closing it is how you get
        // the space back, not how you throw the tabs away.
        const remembered = state.browserMemory.get(sessionId)
        next.set(
          sessionId,
          remembered ?? { tabs: [{ url: DEFAULT_BROWSER_URL }], activeTab: 0, sessionId }
        )
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

  addBrowserTab: (paneId, url, opts) =>
    set((state) => {
      const existing = state.browserPanes.get(paneId)
      if (!existing) return {}
      const normalized =
        url === undefined ? DEFAULT_BROWSER_URL : opts?.trusted ? url : normalizeUrl(url)
      if (!normalized) return {}
      const next = new Map(state.browserPanes)
      const tabs = [...existing.tabs, { url: normalized }]
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

  syncBrowserTab: (paneId, index, seen) =>
    set((state) => {
      const existing = state.browserPanes.get(paneId)
      if (!existing || index < 0 || index >= existing.tabs.length) return {}
      const tab = existing.tabs[index]

      // A new page drops the old page's title. Titles only ever arrived and
      // never left, so a guest that navigated to a page with no <title> — whose
      // report is dropped as unset — kept advertising the *previous* page's
      // name against the new url, indefinitely.
      // Against where the tab effectively was, not just its last observation. A
      // tab that has reported a title but not yet a url still *is* somewhere —
      // its intent — and comparing against an absent `liveUrl` made the first
      // navigation report look like a move even when it named that same page,
      // clearing a title the page had just set.
      const movedOn = seen.url !== undefined && seen.url !== (tab.liveUrl ?? tab.url)
      const liveUrl = seen.url ?? tab.liveUrl
      const title = seen.title ?? (movedOn ? undefined : tab.title)
      // A guest re-reports the same url on every load event. Rebuilding the map
      // for no change would rerender the pane — and remount nothing, but the
      // work is pure waste on a page that reloads itself.
      if (liveUrl === tab.liveUrl && title === tab.title) return {}

      const tabs = [...existing.tabs]
      tabs[index] = { url: tab.url, ...(liveUrl ? { liveUrl } : {}), ...(title ? { title } : {}) }
      const next = new Map(state.browserPanes)
      next.set(paneId, { ...existing, tabs })
      // Not persisted: `savePanes` writes intent only, so an observation is
      // never worth a write. Leaving it out also keeps a page that reloads on a
      // timer from touching localStorage forever.
      return { browserPanes: next }
    }),

  openDevicePane: (sessionId, device) =>
    set((state) => {
      const existing = state.devicePanes.get(sessionId)
      if (existing && existing.udid === device.udid && existing.name === device.name) return {}
      const next = new Map(state.devicePanes)
      next.set(sessionId, device)
      rememberDeviceRequest(sessionId, device)
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
      // Surfaced rather than swallowed: the likeliest failure is another
      // session holding the device, and that message names the holder.
      if (!claimed.ok) return claimed
      get().openDevicePane(sessionId, { udid: claimed.udid, name: claimed.name })
      return null
    } catch (e) {
      // Everything the claim does not have a name for -- the channel itself
      // failing, an older main process. Reported as a boot failure because that
      // is the one outcome that means "this device, this time, did not work".
      return { reason: 'boot-failed', message: e instanceof Error ? e.message : String(e) }
    }
  },

  restoreDevicePanes: async () => {
    const requests = loadDeviceRequests()
    const refused: DeviceRestoreRefusal[] = []
    if (requests.size === 0) return refused
    // One at a time. Each claim boots a simulator, and `simctl bootstatus`
    // blocks until it is up -- taking six together is six simulators starting
    // in the same instant on a machine that has just finished launching.
    for (const [sessionId, device] of requests) {
      // Read per iteration: this loop awaits a boot each time round, and the
      // board can gain a session while it does.
      if (!get().terminals.has(sessionId)) continue
      if (get().devicePanes.has(sessionId)) continue
      const failure = await get().claimAndOpenDevicePane(sessionId, device)
      if (!failure) continue
      // A simulator that has been deleted, or a record carried to another
      // machine, is not something the person did or can act on. Forgotten
      // rather than reported, so it stops being tried on every launch.
      if (failure.reason === 'gone') {
        forgetDeviceRequest(sessionId)
        continue
      }
      // The rest are contested or broken, and the record is kept: the device is
      // still the one this session was working against, and the next launch --
      // after the other Vorn has quit -- should try it again.
      refused.push({ sessionId, device, failure })
    }
    return refused
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
      forgetDeviceRequest(sessionId)
      return {
        devicePanes: next,
        ...dropDeviceSplit(state, sessionId),
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
    // Intent travels; observation does not. The card mounts a *fresh* guest on
    // `url`, so carrying `liveUrl` and `title` across would have the card's
    // address bar assert the redirected location while its guest is still at
    // the original one — and for a page that always redirects, permanently.
    // The new guest reports for itself the moment it navigates.
    const tab = { url: pane.tabs[index].url }
    const cardId = nextCardId(pane.sessionId)
    // Out of the strip before into the card: leaving it in both would mount two
    // guests on one url, each with its own scroll position and half-typed form,
    // and closing either would look like the page had refused to go away.
    get().closeBrowserTab(paneId, index)
    set((state) => {
      const next = new Map(state.browserPanes)
      next.set(cardId, { tabs: [tab], activeTab: 0, sessionId: pane.sessionId })
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
      //
      // Where each guest actually is, not where it was first sent: a tab that
      // followed a link would otherwise snap back to the page it started on.
      const [first, ...rest] = browser.tabs.map(tabUrl)
      // Trusted: these urls were vetted when they first entered the card, and a
      // `file:` one cannot be re-checked here — the renderer has no filesystem.
      // Left untrusted, returning a card sitting on an in-root file page would
      // no-op every add below and then close the card holding the only copy.
      const before = get().browserPanes.get(browser.sessionId)?.tabs.length ?? 0
      if (state.browserPanes.has(browser.sessionId))
        state.addBrowserTab(browser.sessionId, first, { trusted: true })
      // If that browser is closed, opening it on this page is the landing spot —
      // a tab with nowhere to go would leave the card the only thing holding it,
      // and it is about to close.
      else state.openBrowserPane(browser.sessionId, first, { trusted: true })

      // If the landing failed — a url that no longer normalizes, from a
      // corrupted store — every add below would no-op and the close would take
      // the whole card with it. Keep the card instead; it is the only copy.
      // Landing has to have *added* something: an existing browser already has
      // tabs, so its mere presence proves nothing about whether this one landed.
      const landed = get().browserPanes.get(browser.sessionId)
      if (!landed || landed.tabs.length === before) return

      const firstIndex = landed.tabs.length - 1
      for (const url of rest) state.addBrowserTab(browser.sessionId, url, { trusted: true })
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

  setActiveTabId: (id) => {
    saveView({ activeTabId: id })
    set({ activeTabId: id })
  },

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
