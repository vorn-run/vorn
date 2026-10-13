import { Terminal, type ITerminalAddon } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import '@xterm/xterm/css/xterm.css'
import {
  attachCommandBlocks,
  getCommandBlocks,
  jumpToCommand,
  markSeededFromServer
} from './command-blocks'
import { chooseAnchor, readScrollAnchor, resolveAnchor, writeScrollAnchor } from './scroll-anchor'
import type { BufferMetrics } from './spine-layout'
import { clearBlockLog } from './block-log'
import { startOverlaySync, type OverlaySync } from './overlay-sync'
import { TERMINAL_BACKGROUND } from '../../shared/surface'
import type { TerminalData, TerminalPresence } from '@vornrun/shared/protocol'
import type { RecordCursor } from '@vornrun/shared/types'
import { decodeTerminalFrameV2, frameResume } from '@vornrun/shared/terminal-frame'
import { swallowQueries } from './vornd-replies'
import { resizeInOrder } from './stream-resize'
import { fitGrid } from './grid-fit'

interface TerminalEntry {
  term: Terminal
  fitAddon: FitAddon
  persistentWrapper: HTMLDivElement | null
  activeSlot: HTMLElement | null
  /** When set, the grid is fitted to this box and the slot is only the window onto it. */
  fitElement: HTMLElement | null
  lastAppliedRect: { top: number; left: number; width: number; height: number } | null
  /** The rows shown of the grid, as last written: top, rows above it, rows in it, the box under it. */
  lastWindow: { top: number; offset: number; shown: number; boxHeight: number } | null
  /** The cell as xterm laid it out at the last fit, for the window and the hold. */
  cell: { width: number; height: number } | null
  lastSyncedCols: number
  lastSyncedRows: number
  /** The hold before a moved box is taken as the new size, and the deadline a box that keeps moving cannot push past. */
  resizeTimer: ReturnType<typeof setTimeout> | null
  resizeDeadline: ReturnType<typeof setTimeout> | null
  /** The font the user chose; a pane drawing a grid larger than its box scales below it. */
  userFont: number
  /** The grid is larger than the box even at the smallest font, and the pane pans across it. */
  panned: boolean
  /**
   * This pane's own number in what it tells vornd. Every window of the desktop
   * shares one connection, so vornd tells two panes of one session apart by it.
   */
  pane: number
  /** The cells the box holds at the user's font, for the box size it was counted in. */
  room: { width: number; height: number; cols: number; rows: number } | null
  _loadRenderer?: (() => void) | null
  _gpuAddon?: { dispose(): void } | null
  _disposeCommandBlocks?: (() => void) | null
  _disposeScrollAnchor?: (() => void) | null
  /** Whether this terminal has already been seeded from the server. */
  _hydrated?: boolean
}

/** data attribute on the persistent wrapper, read by TerminalHost for event delegation. */
export const TERMINAL_ID_ATTR = 'data-terminal-id'

/** How long the scroll has to stand still before an anchor is worth a write. */
const ANCHOR_SETTLE_MS = 250

/**
 * Where a hidden wrapper waits: outside the viewport, at the size it last had.
 *
 * `visibility: hidden` alone leaves xterm drawing every frame of output into a
 * canvas nobody sees, because the IntersectionObserver which xterm uses to pause
 * its renderer still counts a hidden element as on screen. Out here it does
 * not, so a terminal in another tab or view parses its output but draws none of
 * it, and repaints once when it is shown again. Moving rather than `display:
 * none` keeps its size, so the cell xterm measured stays right.
 */
const PARKED_LEFT = '-100000px'

const registry = new Map<string, TerminalEntry>()

/**
 * Terminals currently being written a screen from the past.
 *
 * A replayed screen is a recording, and recordings contain the questions the
 * old program asked its terminal -- who are you, where is the cursor, what
 * colour is the background. Written into a real emulator those are asked again,
 * and this one answers, because answering is what a terminal does. The answers
 * go down the pty as though they had been typed: the shell echoes them, tries to
 * run them, and the pane fills with `rgb:d4d4/d4d4/d8d8` and `execute:`.
 *
 * The program that asked is gone and nothing is waiting for a reply, so during
 * a seed there is nobody to answer and the replies are dropped. Only during the
 * seed -- a live program asking the same question is owed a real answer.
 *
 * The server's own screen model was guarded against this from the start; the
 * client had the same hole and no seeded screen to fall into it until this
 * branch.
 */
const seeding = new Set<string>()
const readyCallbacks = new Map<string, Set<() => void>>()

/**
 * One flush of a session's output; see `PtyManager.flushSeq` for `seq`.
 *
 * From vornd, `seq` is the last record the chunk holds and `cursor` where the
 * screen ends once it is applied; a resize of the session is a chunk too, at
 * its place among them, with its record as `seq`.
 */
type Chunk =
  | Pick<TerminalData, 'data' | 'seq' | 'cursor'>
  | { resize: { cols: number; rows: number }; seq: number }

/**
 * Terminals vornd streams, and where each one's screen ends in its session's
 * record log. A reconnect attaches them again from there, so they carry on
 * without a snapshot while vornd still has what follows.
 */
const vorndStreams = new Map<string, { cursor: RecordCursor | null }>()

/**
 * Where a pane showing a session vornd holds stands toward the session's size.
 *
 * vornd decides that size, from who is typing (the Terminal State Protocol's
 * size rule): the pane only reports what fits in it and whether it is in use,
 * and draws the session's grid whatever its size, scaled and then panned,
 * never clipped and never resized here. "Fit to this device" and the lock are
 * the two ways to ask for the size outright.
 */
interface Sizing {
  /** This connection's name in vornd, from the attach answer. */
  client: string | null
  /** Who the size follows, as the last resize named it. */
  owner: string | null
  /** This pane locked the size. */
  locked: boolean
  /** What fits in the pane at the user's font, as last reported. */
  viewport: { cols: number; rows: number } | null
  /** As last reported. */
  presence: TerminalPresence | null
  /** Until when the pane counts as in use: typed into or scrolled a moment ago. */
  activeUntil: number
  activeTimer: ReturnType<typeof setTimeout> | null
}

const sizing = new Map<string, Sizing>()

/**
 * A number for a new pane, unlikely to be any other pane's in any window: the
 * windows have no counter in common, and vornd only needs them distinct.
 */
function newPaneId(): number {
  return Math.floor(Math.random() * 2 ** 48) + 1
}

/** The pane's number, for a session vornd holds; undefined for any other. */
function paneOf(id: string): number | undefined {
  return vorndStreams.has(id) ? registry.get(id)?.pane : undefined
}

/** How long input or scrolling keeps a pane `active` rather than `watching`. */
const ACTIVE_MS = 5_000

function sizingFor(id: string): Sizing {
  let s = sizing.get(id)
  if (!s) {
    s = {
      client: null,
      owner: null,
      locked: false,
      viewport: null,
      presence: null,
      activeUntil: 0,
      activeTimer: null
    }
    sizing.set(id, s)
  }
  return s
}

function dropSizing(id: string): void {
  const s = sizing.get(id)
  if (s?.activeTimer) clearTimeout(s.activeTimer)
  if (sizing.delete(id)) notifySizing()
}

const sizingListeners = new Set<() => void>()

function notifySizing(): void {
  for (const cb of sizingListeners) {
    try {
      cb()
    } catch {
      // one listener's failure is not another's
    }
  }
}

/**
 * The session's size changed under this pane: remembered as the size last
 * agreed, so the pane's next fit tells the session its own size when that
 * differs, rather than finding the two equal and leaving the grid and the
 * pty at different sizes.
 */
function sessionSized(id: string, term: Terminal, cols: number, rows: number): void {
  const entry = registry.get(id)
  if (!entry || entry.term !== term) return
  entry.lastSyncedCols = cols
  entry.lastSyncedRows = rows
  // A new grid is drawn at whatever font fits it in this pane.
  if (vorndStreams.has(id) && entry.term.element && entry.lastAppliedRect) fitHeldGrid(entry, id)
}

/** Moves a vornd stream's cursor past a chunk that was just applied. */
function advance(id: string, chunk: Chunk): void {
  const stream = vorndStreams.get(id)
  if (!stream) return
  if ('resize' in chunk) {
    if (stream.cursor) stream.cursor = { ...stream.cursor, nextRseq: chunk.seq + 1 }
  } else if (chunk.cursor) {
    stream.cursor = chunk.cursor
  }
}

/** The chunks held behind a seed: text joined, bytes as they came, since joining bytes means copying them. */
function writeChunks(id: string, term: Terminal, chunks: readonly Chunk[]): void {
  let text = ''
  for (const chunk of chunks) {
    if ('resize' in chunk) {
      if (text) {
        term.write(text)
        text = ''
      }
      // Output after a resize record was written for the new size.
      const { cols, rows } = chunk.resize
      resizeInOrder(term, cols, rows, () => sessionSized(id, term, cols, rows))
      advance(id, chunk)
      continue
    }
    advance(id, chunk)
    if (typeof chunk.data === 'string') {
      text += chunk.data
      continue
    }
    if (text) {
      term.write(text)
      text = ''
    }
    term.write(chunk.data)
  }
  if (text) term.write(text)
}

/**
 * Sessions being seeded right now.
 *
 * A pane that did not create its terminal has to be given what the terminal
 * already shows before it applies anything live, and the two must not cross. So
 * live chunks are held from before the seed is asked for until after it has been
 * written -- then the ones the seed already contains are dropped by their number
 * and the rest are applied in order.
 */
interface Hydration {
  /** Live chunks arriving while the seed is in flight. */
  held: Chunk[]
  /** The seed itself, so concurrent callers join it rather than starting another. */
  done: Promise<void>
}

const hydrating = new Map<string, Hydration>()

/** Live output goes straight to xterm, which draws on its own frame; holding it for one of ours only added a frame. */
function receive(id: string, chunk: Chunk): void {
  const hydration = hydrating.get(id)
  if (hydration) {
    hydration.held.push(chunk)
    return
  }
  const entry = registry.get(id)
  if (!entry) return
  if ('resize' in chunk) {
    writeChunks(id, entry.term, [chunk])
    return
  }
  entry.term.write(chunk.data)
  advance(id, chunk)
  // While a shrinking box waits out its hold, its window shows the rows around
  // the cursor, which output moves without moving any layout.
  if (entry.resizeTimer) overlaySync?.request()
}

let removeGlobalDataListener: (() => void) | null = null
let removeResyncListener: (() => void) | null = null
let removeStreamListeners: Array<() => void> = []

export function initGlobalDataListener(): void {
  if (removeGlobalDataListener) return
  removeGlobalDataListener = window.api.onTerminalData(({ id, data, seq }) =>
    receive(id, { data, seq })
  )
  // Optional for a surface older than the notification, as attach is below.
  removeResyncListener = window.api.onTerminalResync?.(({ id }) => void resyncTerminal(id)) ?? null
  // vornd's streams: frames read here, resizes in order with them, and a new
  // connection attaching every one of them again from its cursor.
  const frames = window.api.onTerminalFrame?.((bytes) => {
    const frame = decodeTerminalFrameV2(bytes)
    if (frame)
      receive(frame.id, { data: frame.data, seq: frame.lastRseq, cursor: frameResume(frame) })
  })
  const resized = window.api.onTerminalResized?.(({ id, cols, rows, rseq, owner }) => {
    const s = sizing.get(id)
    if (s && owner !== undefined) {
      s.owner = owner ?? null
      notifySizing()
    }
    receive(id, { resize: { cols, rows }, seq: rseq })
  })
  const reconnected = window.api.onTerminalReconnected?.(() => {
    for (const id of vorndStreams.keys()) void reattachTerminal(id)
  })
  // A window in the background is away for every session it shows.
  const visibility = (): void => {
    for (const id of sizing.keys()) reportPresence(id)
  }
  document.addEventListener('visibilitychange', visibility)
  const hidden = (): void => document.removeEventListener('visibilitychange', visibility)
  removeStreamListeners = [frames, resized, reconnected, hidden].filter(
    (off): off is () => void => typeof off === 'function'
  )
}

export function disposeGlobalDataListener(): void {
  removeGlobalDataListener?.()
  removeGlobalDataListener = null
  removeResyncListener?.()
  removeResyncListener = null
  for (const off of removeStreamListeners) off()
  removeStreamListeners = []
  hydrating.clear()
  seeding.clear()
  vorndStreams.clear()
  for (const id of [...sizing.keys()]) dropSizing(id)
}

/**
 * Show a terminal this pane did not create.
 *
 * Ordering is the whole of it, and getting it wrong is invisible until somebody
 * reads their scrollback:
 *
 *   1. Start holding live chunks. Before asking, not after -- anything that
 *      arrives while the request is in flight belongs after the seed, and there
 *      is no way to recover it once it has been written ahead of one.
 *   2. Ask. The answer carries the scrollback and the flush it reflects, read on
 *      the server in a single tick so the two cannot disagree.
 *   3. Write the seed, then the held chunks numbered above it. Anything at or
 *      below that number is already in the seed; applying it again would print
 *      those bytes twice.
 *
 * The seed goes straight to the terminal rather than through the batch above, so
 * it never reaches a status handler. Replaying a screen must not ring a bell an
 * agent rang an hour ago.
 *
 * Idempotent, and shared: a second window, a reconnect and a re-render all land
 * here, and one already in flight is joined rather than started again. A
 * terminal seeded twice has its scrollback twice.
 */
export function hydrateTerminal(
  terminalId: string,
  { replace = false, resume = false }: { replace?: boolean; resume?: boolean } = {}
): Promise<void> {
  const already = hydrating.get(terminalId)
  if (already) return already.done

  const entry = registry.get(terminalId)
  if (!entry || entry._hydrated) return Promise.resolve()
  // Absent on an older client surface, and in tests that mock a smaller one. A
  // terminal with nothing to be seeded from simply carries on live.
  if (typeof window.api?.attachTerminal !== 'function') return Promise.resolve()
  entry._hydrated = true

  const state: Hydration = { held: [], done: Promise.resolve() }
  hydrating.set(terminalId, state)

  /** Everything held, in order, after the seed. */
  const flushHeld = (above = -1): void => {
    // A chunk whose sequence cannot be compared cannot be deduplicated, and the
    // choice is then between showing it twice and not showing it at all. Twice
    // is visible and a person can see what happened; dropping it is silent, and
    // losing output is the failure this whole mechanism exists to prevent. The
    // protocol requires `seq`, so this only arises against a server that is not
    // keeping to it -- which is exactly when guessing is the wrong thing to do.
    const kept = state.held.filter(
      (chunk) =>
        !Number.isFinite(chunk.seq) || !Number.isFinite(above) || (chunk.seq as number) > above
    )
    if (!kept.length) return
    writeChunks(terminalId, entry.term, kept)
  }

  /**
   * Whether the pane this seed was started for is still the one holding the id.
   *
   * `destroyTerminal` can run while the attach is in flight, and the id can be
   * mounted again straight after -- a view swap, a pane closed and reopened.
   * The closure still holds the old entry, so the seed would write into a
   * terminal that has been disposed. xterm does not object to that (it neither
   * throws nor drops the callback, which is worth saying because it means the
   * damage is silent), but `flushHeld` also feeds whatever status handler is
   * registered for the id -- and by then that belongs to the new pane, which
   * would be handed the old pane's bytes and read a status out of them.
   */
  const stillOurs = (): boolean => registry.get(terminalId) === entry

  state.done = (async () => {
    try {
      // A terminal vornd streams resumes from where its screen ends, when
      // asked to: vornd continues from there if it still can.
      const cursor = resume ? (vorndStreams.get(terminalId)?.cursor ?? undefined) : undefined
      const answer = await window.api.attachTerminal(terminalId, cursor)
      const { data, seq, live } = answer
      if (!stillOurs()) return
      const fromVornd = answer.replies === 'vornd'
      if (fromVornd) {
        vorndStreams.set(terminalId, { cursor: answer.cursor ?? null })
        const s = sizingFor(terminalId)
        s.client = answer.client ?? s.client
        // A new connection reports afresh.
        s.viewport = null
        s.presence = null
        notifySizing()
      } else {
        vorndStreams.delete(terminalId)
        dropSizing(terminalId)
      }
      // vornd hears what fits here and whether anyone is looking, never a
      // size; a new connection hears it afresh, a continued stream included.
      const reportHeld = (): void => {
        if (!fromVornd) return
        if (entry.term.element && entry.lastAppliedRect) fitHeldGrid(entry, terminalId)
        reportPresence(terminalId)
      }
      if (answer.continued) {
        // Nothing missed: the screen stays, and what follows the cursor applies.
        flushHeld(seq)
        reportHeld()
        return
      }
      // vornd's snapshot is drawn for the session's size.
      if (fromVornd && answer.cols && answer.rows) {
        const cols = answer.cols
        const rows = answer.rows
        resizeInOrder(entry.term, cols, rows, () =>
          sessionSized(terminalId, entry.term, cols, rows)
        )
      }
      // A resync replaces the screen only with something. A terminal that
      // ended while this window was behind comes back empty, and what is on
      // screen here is then more than the server still has. A stream that
      // could not continue is replaced by vornd's snapshot the same way.
      if ((replace || (fromVornd && resume)) && data) {
        entry.term.reset()
        // The seed replays the shell's command marks, and the log would take
        // every command in it a second time. It is rebuilt from the seed, as
        // the log of a pane that did not create its terminal is.
        clearBlockLog(terminalId)
      }
      if (data) {
        // Cleared from the write callback, which xterm runs once these bytes
        // have been parsed -- so it covers every reply they provoke and nothing
        // after them.
        seeding.add(terminalId)
        entry.term.write(data, () => seeding.delete(terminalId))
        // This screen is now in the terminal and nowhere else. Said out loud so
        // the first finished command lifts it into the block log rather than
        // clearing it away.
        markSeededFromServer(terminalId)
      }
      flushHeld(seq)
      // An empty write, to get a callback after everything above it has been
      // parsed. The blocks the anchor names are built by that parsing, and the
      // held chunks scroll the pane back to the bottom on their way through, so
      // there is no earlier point at which the restore would survive. Out of the
      // parse loop itself, which is where xterm runs write callbacks.
      //
      // Only when there is a position to put back, so a terminal that was left
      // at the bottom -- which is nearly all of them -- puts nothing extra
      // through the queue at all.
      if (readScrollAnchor(terminalId)) {
        entry.term.write('', () => setTimeout(() => restoreScrollAnchor(terminalId), 0))
      }
      // The one moment a pane learns the truth about its session. A window
      // opened onto a terminal that died while it was closed has no start-up
      // reconciliation to tell it -- this is where it finds out.
      if (live === false) reportNotLive?.(terminalId)
      if (live === true) reportLive?.(terminalId)
      reportHeld()
    } catch (err) {
      console.error('[terminal] could not attach', terminalId, err)
      if (!stillOurs()) return
      // Held chunks are still the truth about what happened; let them through
      // rather than losing them to a failed seed, and allow another try.
      flushHeld()
      entry._hydrated = false
    } finally {
      hydrating.delete(terminalId)
      if (
        stillOurs() &&
        !vorndStreams.has(terminalId) &&
        entry.term.element &&
        entry.lastAppliedRect
      )
        fitNow(entry, terminalId)
    }
  })()
  return state.done
}

/**
 * Start a terminal again from the server's screen, after output was withheld.
 *
 * The server stops sending a window output it cannot keep up with, and says so
 * once the window has caught up. What it skipped is gone from this side, so the
 * screen here is wrong in a way no later output repairs: it is cleared and
 * seeded again, exactly as a pane that did not create its terminal is.
 *
 * After any seed already in flight, rather than joining it: that one may have
 * been asked for before the gap, and its answer would not cover it.
 */
export function resyncTerminal(terminalId: string): Promise<void> {
  const inFlight = hydrating.get(terminalId)?.done ?? Promise.resolve()
  return inFlight.then(() => {
    const entry = registry.get(terminalId)
    if (!entry) return
    entry._hydrated = false
    return hydrateTerminal(terminalId, { replace: true })
  })
}

/**
 * Attach a terminal vornd streams again after the connection came back, from
 * where its screen ends. vornd continues the stream from there while it still
 * holds what follows, through a vornd restart included; otherwise it answers
 * with a snapshot, which replaces the screen.
 */
export function reattachTerminal(terminalId: string): Promise<void> {
  const inFlight = hydrating.get(terminalId)?.done ?? Promise.resolve()
  return inFlight.then(() => {
    const entry = registry.get(terminalId)
    if (!entry || !vorndStreams.has(terminalId)) return
    // The old connection's lock and ownership went with it: vornd released
    // them when it closed.
    const s = sizing.get(terminalId)
    if (s) {
      s.locked = false
      s.owner = null
      notifySizing()
    }
    entry._hydrated = false
    return hydrateTerminal(terminalId, { resume: true })
  })
}

/**
 * Told when an attach finds nothing running behind a terminal.
 *
 * Set once at start-up by the app, which turns it into the pane's ended state.
 * Kept as a reporter rather than an import so this module stays about terminals
 * and knows nothing about the store.
 */
type NotLiveReporter = (terminalId: string) => void
let reportNotLive: NotLiveReporter | null = null
/** Told when an attach found a process still running: a warm restore, if the board did not start it. */
let reportLive: NotLiveReporter | null = null

export function setLiveReporter(fn: NotLiveReporter | null): void {
  reportLive = fn
}

export function setNotLiveReporter(fn: NotLiveReporter | null): void {
  reportNotLive = fn
}

/**
 * Optional keystroke redirect, wired at app startup. Returning true means the
 * keystroke was claimed (e.g. focus moved to the intent bar so the character
 * lands there); xterm then ignores the event without preventing the browser
 * default, which is what delivers the character to the newly focused input.
 */
type KeyRedirectHandler = (terminalId: string, ev: KeyboardEvent) => boolean
let keyRedirectHandler: KeyRedirectHandler | null = null

export function setKeyRedirectHandler(handler: KeyRedirectHandler | null): void {
  keyRedirectHandler = handler
}

const TERM_OPTIONS = {
  cursorBlink: true,
  fontSize: 13,
  fontFamily: 'JetBrains Mono, Menlo, Monaco, Consolas, Liberation Mono, Courier New, monospace',
  theme: {
    background: TERMINAL_BACKGROUND,
    foreground: '#d4d4d8',
    cursor: '#d4d4d8',
    selectionBackground: '#3f3f46',
    black: '#27272a',
    red: '#ef4444',
    green: '#22c55e',
    yellow: '#eab308',
    blue: '#3b82f6',
    magenta: '#a855f7',
    cyan: '#06b6d4',
    white: '#d4d4d8',
    brightBlack: '#52525b',
    brightRed: '#f87171',
    brightGreen: '#4ade80',
    brightYellow: '#facc15',
    brightBlue: '#60a5fa',
    brightMagenta: '#c084fc',
    brightCyan: '#22d3ee',
    brightWhite: '#fafafa'
  },
  scrollback: 2000,
  allowProposedApi: true
}

/** Allow overriding default font size from config */
let configFontSize = 13

export function setDefaultFontSize(size: number): void {
  configFontSize = size
}

/** Returns the effective font size (respects user config, no forced minimum). */
export function getEffectiveFontSize(size?: number): number {
  return size ?? configFontSize
}

const rendererIsMac = navigator.platform.toUpperCase().includes('MAC')

function createTerminalEntry(terminalId: string): TerminalEntry {
  // A write callback that never ran -- a pane torn down mid-seed -- would
  // otherwise leave this id suppressed for as long as the window lives.
  seeding.delete(terminalId)
  const term = new Terminal({ ...TERM_OPTIONS, fontSize: getEffectiveFontSize() })
  const fitAddon = new FitAddon()
  term.loadAddon(fitAddon)

  // Clickable links — Cmd+click (Mac) / Ctrl+click (Windows/Linux) opens in browser
  term.loadAddon(
    new WebLinksAddon((event, uri) => {
      const mod = rendererIsMac ? event.metaKey : event.ctrlKey
      if (mod) window.api.openExternal(uri)
    })
  )

  // Let app-level shortcuts pass through instead of being consumed by xterm
  term.attachCustomKeyEventHandler((e) => {
    if (e.type === 'keydown' && keyRedirectHandler?.(terminalId, e)) return false
    // A held size goes before a keystroke, so what is typed reaches the program after the SIGWINCH.
    const held = registry.get(terminalId)
    if (e.type === 'keydown' && held?.resizeTimer) fitNow(held, terminalId)
    if (e.type === 'keydown') markActive(terminalId)

    const mod = rendererIsMac ? e.metaKey : e.ctrlKey
    if (!mod) return true

    // Jump between command blocks (shell-integration markers)
    if (e.type === 'keydown' && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      jumpToCommand(terminalId, term, e.key === 'ArrowUp' ? -1 : 1)
      e.preventDefault()
      return false
    }

    if (!rendererIsMac && e.type === 'keydown') {
      const key = e.key.toLowerCase()

      // Copy on Windows/Linux — Ctrl+C copies when text is selected,
      // otherwise falls through so xterm sends SIGINT. Ctrl+Shift+C always copies.
      if (key === 'c' && (e.shiftKey || term.hasSelection())) {
        if (term.hasSelection()) {
          navigator.clipboard.writeText(term.getSelection())
          term.clearSelection()
        }
        e.preventDefault()
        return false
      }

      // Paste on Windows/Linux — Ctrl+V / Ctrl+Shift+V: xterm intercepts Ctrl+V
      // as a control character (\x16) instead of triggering the browser paste event.
      // Read clipboard manually and use term.paste() for bracketed-paste support.
      // preventDefault() is critical to stop the browser from also firing a native
      // paste event, which would cause xterm to paste the text a second time.
      if (key === 'v') {
        e.preventDefault()
        navigator.clipboard.readText().then((text) => {
          if (text) term.paste(text)
        })
        return false
      }
    }

    const passthrough = ['w', '[', ']', 'k', 'n', 'o', 'b', ',', '/']
    if (passthrough.includes(e.key.toLowerCase())) return false
    return true
  })

  const mountAddon = (make: () => ITerminalAddon): void => {
    // Re-check under the await — the terminal may have been destroyed
    // while the dynamic import was in flight, and a concurrent load may
    // have already installed an addon.
    const e = registry.get(terminalId)
    if (!e || !e.term.element || e._gpuAddon) return
    const addon = make()
    term.loadAddon(addon)
    e._gpuAddon = addon
    term.refresh(0, term.rows - 1)
    // The first fit ran against the DOM renderer's idea of a cell. A GPU
    // renderer measures the font itself and usually lands on a slightly
    // narrower cell, so the column count chosen a moment ago is now too low —
    // and nothing else recomputes it, because the wrapper's size has not
    // changed. That is the whole reason a terminal sat with a band of unused
    // width down its right edge until the window was resized: resizing was the
    // only thing that ever asked it to fit again.
    fitTerminal(terminalId)
  }
  // Terminal fallback — if even canvas fails to load, there's no further
  // fallback, so swallow the error here instead of propagating an unhandled
  // rejection up through the WebGL error paths.
  const loadCanvas = (): void => {
    import('@xterm/addon-canvas')
      .then(({ CanvasAddon }) => mountAddon(() => new CanvasAddon()))
      .catch(() => {})
  }
  const loadRenderer = (): void => {
    const current = registry.get(terminalId)
    if (!current || current._gpuAddon) return
    import('@xterm/addon-webgl')
      .then(({ WebglAddon }) => {
        try {
          mountAddon(() => new WebglAddon())
        } catch {
          loadCanvas()
        }
      })
      .catch(() => {
        loadCanvas()
      })
  }

  // vornd answers the queries of the sessions it holds; this terminal must not.
  swallowQueries(term, () => vorndStreams.has(terminalId))

  // Forward keystrokes to pty
  term.onData((data) => {
    if (seeding.has(terminalId)) return
    // A focus report is the terminal's, not the person's.
    if (data !== '\x1b[I' && data !== '\x1b[O') markActive(terminalId)
    const pane = paneOf(terminalId)
    if (pane === undefined) window.api.writeTerminal(terminalId, data)
    else window.api.writeTerminal(terminalId, data, pane)
  })

  const disposeCommandBlocks = attachCommandBlocks(terminalId, term)

  const entry: TerminalEntry = {
    term,
    fitAddon,
    persistentWrapper: null,
    activeSlot: null,
    fitElement: null,
    lastAppliedRect: null,
    lastWindow: null,
    cell: null,
    lastSyncedCols: 0,
    lastSyncedRows: 0,
    resizeTimer: null,
    resizeDeadline: null,
    userFont: getEffectiveFontSize(),
    panned: false,
    room: null,
    pane: newPaneId()
  }

  entry._loadRenderer = loadRenderer
  entry._disposeCommandBlocks = disposeCommandBlocks

  registry.set(terminalId, entry)

  // After the entry is in the registry, which is what `onTerminalScroll` reads.
  entry._disposeScrollAnchor = trackScrollAnchor(terminalId)

  // Every terminal is seeded, not only the ones adopted from a previous run.
  //
  // Started here rather than left to whoever created the pane, and started the
  // moment the entry exists, because the hold that makes seeding safe has to be
  // in place before the first live chunk is written. A terminal this client
  // created a moment ago is seeded from an empty scrollback and nothing happens;
  // one that was already running gets everything it missed. Two paths through
  // one door beats a flag saying which door this was.
  void hydrateTerminal(terminalId)

  const cbs = readyCallbacks.get(terminalId)
  if (cbs) {
    cbs.forEach((cb) => cb())
    readyCallbacks.delete(terminalId)
  }

  notifyRegistryChange()

  return entry
}

// Every xterm is opened into a persistent wrapper div that lives in the
// singleton TerminalHost and never moves — reparenting would interrupt
// the WebGL context and produce flicker on view switches.

let hostRoot: HTMLElement | null = null
const registryChangeListeners = new Set<() => void>()
let cachedTerminalIds: string[] | null = null

function notifyRegistryChange(): void {
  cachedTerminalIds = null
  overlaySync?.slotsChanged()
  overlaySync?.request()
  for (const cb of registryChangeListeners) {
    try {
      cb()
    } catch {
      // listener threw — isolate to not block other subscribers
    }
  }
}

function ensurePersistentWrapper(entry: TerminalEntry, terminalId: string): HTMLDivElement {
  if (entry.persistentWrapper) return entry.persistentWrapper
  const wrapper = document.createElement('div')
  wrapper.setAttribute(TERMINAL_ID_ATTR, terminalId)
  wrapper.style.position = 'fixed'
  wrapper.style.top = '0'
  wrapper.style.left = PARKED_LEFT
  wrapper.style.width = '0'
  wrapper.style.height = '0'
  wrapper.style.visibility = 'hidden'
  wrapper.style.pointerEvents = 'none'
  // The grid follows the box a beat behind it, so for that beat it may be the larger of the two.
  wrapper.style.overflow = 'hidden'
  // Scrolling a session's history, or panning across its grid, is using it.
  wrapper.addEventListener('wheel', () => markActive(terminalId), { passive: true })
  entry.persistentWrapper = wrapper
  if (hostRoot) {
    hostRoot.appendChild(wrapper)
    openIntoPersistentWrapper(entry, terminalId)
  }
  return wrapper
}

function openIntoPersistentWrapper(entry: TerminalEntry, terminalId: string): void {
  if (entry.term.element) return
  const wrapper = entry.persistentWrapper
  if (!wrapper || !wrapper.parentElement) return
  entry.term.open(wrapper)
  entry._loadRenderer?.()
  // A font that arrives after the terminal opened changes the cell width under
  // a column count already chosen, the same way a renderer swap does. Resolved
  // immediately when nothing is pending, so this costs a microtask in the
  // common case.
  void document.fonts?.ready.then(() => fitTerminal(terminalId))
}

/**
 * Attach the singleton TerminalHost's root element. Wrappers are appended
 * here; passing null detaches (wrappers remain in the DOM until destroy).
 */
export function setHostRoot(root: HTMLElement | null): void {
  hostRoot = root
  if (!root) return
  for (const [id, entry] of registry) {
    const wrapper = entry.persistentWrapper
    if (wrapper && wrapper.parentElement !== root) {
      root.appendChild(wrapper)
      openIntoPersistentWrapper(entry, id)
      // Re-sync after adoption — registerSlot may have run before the host
      // mounted, setting geometry on a detached wrapper. Now that it's in the
      // DOM and xterm has opened, position + fit correctly.
      syncTerminalOverlay(id)
    }
  }
}

let overlaySync: OverlaySync | null = null

/** Every element a wrapper follows: each terminal's slot, and the box its grid is fitted to. */
function followedElements(): Element[] {
  const out: Element[] = []
  for (const entry of registry.values()) {
    if (entry.activeSlot) out.push(entry.activeSlot)
    if (entry.fitElement) out.push(entry.fitElement)
  }
  return out
}

/**
 * Keep every wrapper on its slot from now until the returned stop is called;
 * see `overlay-sync.ts` for when it looks. One at a time: starting again stops
 * the previous one.
 */
export function startTerminalOverlaySync(root: HTMLElement): () => void {
  overlaySync?.stop()
  const sync = startOverlaySync({
    root,
    ids: getRegisteredTerminalIds,
    sync: syncTerminalOverlay,
    slots: followedElements
  })
  overlaySync = sync
  return () => {
    sync.stop()
    if (overlaySync === sync) overlaySync = null
  }
}

/** Ask the overlay to look again: something moved that it cannot see for itself. */
export function requestOverlaySync(): void {
  overlaySync?.request()
}

/**
 * Register a slot element for a terminal. The wrapper is created (lazily)
 * and will track this slot's bounding rect via syncTerminalOverlay.
 * Last-registered slot wins if multiple slots register for the same id.
 */
export function registerSlot(terminalId: string, slotEl: HTMLElement, fitEl?: HTMLElement): void {
  let entry = registry.get(terminalId)
  if (!entry) entry = createTerminalEntry(terminalId)
  entry.activeSlot = slotEl
  entry.fitElement = fitEl ?? null
  ensurePersistentWrapper(entry, terminalId)
  openIntoPersistentWrapper(entry, terminalId)
  syncTerminalOverlay(terminalId)
  overlaySync?.slotsChanged()
  overlaySync?.request()
}

/**
 * Unregister a slot. No-op if the current active slot is not this element
 * (protects against out-of-order unmounts during rapid view swaps).
 */
export function unregisterSlot(terminalId: string, slotEl: HTMLElement): void {
  const entry = registry.get(terminalId)
  if (!entry || entry.activeSlot !== slotEl) return
  entry.activeSlot = null
  entry.fitElement = null
  syncTerminalOverlay(terminalId)
  overlaySync?.slotsChanged()
}

export function getPersistentWrapper(terminalId: string): HTMLDivElement | null {
  return registry.get(terminalId)?.persistentWrapper ?? null
}

/** True when the wrapper was showing until now. */
function hideWrapper(wrapper: HTMLDivElement, entry: TerminalEntry): boolean {
  const wasShown = wrapper.style.visibility !== 'hidden'
  if (wasShown) {
    wrapper.style.visibility = 'hidden'
    wrapper.style.pointerEvents = 'none'
  }
  wrapper.style.left = PARKED_LEFT
  wrapper.style.clipPath = ''
  entry.lastAppliedRect = null
  entry.lastWindow = null
  if (wasShown) {
    const id = wrapper.getAttribute(TERMINAL_ID_ATTR)
    if (id) reportPresence(id)
  }
  return wasShown
}

/** The cell as xterm laid it out: its screen element over its grid. */
function measureCell(entry: TerminalEntry): void {
  const screen = entry.term.element?.querySelector('.xterm-screen')
  if (!screen || !entry.term.cols || !entry.term.rows) return
  const { width, height } = screen.getBoundingClientRect()
  if (width <= 0 || height <= 0) return
  entry.cell = { width: width / entry.term.cols, height: height / entry.term.rows }
}

/** The rows of the grid that are shown: the slot's, from the top, or a shrinking box's around the cursor while the hold runs. */
function applyWindow(
  entry: TerminalEntry,
  wrapper: HTMLDivElement,
  box: { top: number; height: number },
  win: { top: number; height: number }
): boolean {
  const cell = entry.cell
  const rows = entry.term.rows || 0
  let next: NonNullable<TerminalEntry['lastWindow']>
  if (!cell || !rows) {
    next = { top: box.top, offset: 0, shown: 0, boxHeight: box.height }
  } else {
    const winRows = Math.min(rows, Math.max(1, Math.round(win.height / cell.height)))
    // A panned grid scrolls in its wrapper instead of following the cursor.
    const above =
      entry.fitElement || entry.panned
        ? 0
        : Math.max(0, (entry.term.buffer.active.cursorY || 0) + 1 - winRows)
    const offset = above * cell.height
    next = { top: win.top - offset, offset, shown: winRows * cell.height, boxHeight: box.height }
  }
  const last = entry.lastWindow
  if (
    last &&
    last.top === next.top &&
    last.offset === next.offset &&
    last.shown === next.shown &&
    last.boxHeight === next.boxHeight
  ) {
    return false
  }
  wrapper.style.top = `${next.top}px`
  // clip-path clips hit-testing too, so hidden rows take no clicks.
  wrapper.style.clipPath = next.shown
    ? `inset(${next.offset}px 0 ${Math.max(0, box.height - next.offset - next.shown)}px 0)`
    : ''
  entry.lastWindow = next
  return true
}

/** The window for the rects as they are now. */
function syncWindow(entry: TerminalEntry, wrapper: HTMLDivElement): void {
  const box = entry.lastAppliedRect
  const slot = entry.activeSlot
  if (!box || !slot) return
  if (!entry.fitElement) {
    applyWindow(entry, wrapper, box, box)
    return
  }
  const raw = slot.getBoundingClientRect()
  applyWindow(entry, wrapper, box, { top: Math.round(raw.top), height: Math.round(raw.height) })
}

/** How long a box has to stand still before its size is taken; a slow frame must not lapse it. */
const RESIZE_SETTLE_MS = 100
/** A box that never stands still is fitted anyway by this, so nothing can starve the grid. */
const RESIZE_MAX_WAIT_MS = 400

function clearHold(entry: TerminalEntry): void {
  if (entry.resizeTimer) clearTimeout(entry.resizeTimer)
  if (entry.resizeDeadline) clearTimeout(entry.resizeDeadline)
  entry.resizeTimer = null
  entry.resizeDeadline = null
}

/** Grid, pty and the server's screen model take the box's size at one moment, so none wraps differently from another. */
function fitNow(entry: TerminalEntry, terminalId: string): void {
  clearHold(entry)
  // A hidden wrapper has no box to fit; fitting it would send a 2x1 grid.
  if (!entry.term.element || !entry.lastAppliedRect) return
  // vornd decides the size of the sessions it holds; the pane fits their grid.
  if (vorndStreams.has(terminalId)) {
    fitHeldGrid(entry, terminalId)
    return
  }
  try {
    entry.fitAddon.fit()
  } catch {
    return
  }
  measureCell(entry)
  if (entry.persistentWrapper) syncWindow(entry, entry.persistentWrapper)
  // Until the attach answers, it is not known whether the server or vornd
  // holds the session, and vornd must never hear a size from a pane that
  // only opened it: the pty hears this one once the answer says it may.
  if (hydrating.has(terminalId)) return
  const { cols, rows } = entry.term
  if (cols === entry.lastSyncedCols && rows === entry.lastSyncedRows) return
  entry.lastSyncedCols = cols
  entry.lastSyncedRows = rows
  window.api.resizeTerminal({ id: terminalId, cols, rows })
}

/**
 * A session vornd holds, in a pane: report what fits at the user's font, then
 * draw the session's grid, whatever its size, at the largest font up to the
 * user's that shows it, down to the floor in `grid-fit.ts`, and pan across
 * what still does not fit. The grid itself is never resized here: it changes
 * when the session's resize record arrives.
 */
function fitHeldGrid(entry: TerminalEntry, terminalId: string): void {
  let dims: { cols: number; rows: number } | undefined
  try {
    dims = entry.fitAddon.proposeDimensions()
  } catch {
    return
  }
  if (!dims || !(dims.cols > 0) || !(dims.rows > 0)) return
  const base = entry.userFont
  const current = Number(entry.term.options.fontSize) || base
  const box = entry.lastAppliedRect
  if (!box) return
  // The room is counted at the user's font. A proposal made at that font is
  // exact; one made at a scaled font is converted, which can be a cell short,
  // so it is kept for as long as the box keeps its size rather than counted
  // again at every new grid, which would make the viewport flicker.
  let kept = entry.room
  if (current === base || !kept || kept.width !== box.width || kept.height !== box.height) {
    kept = {
      width: box.width,
      height: box.height,
      cols: Math.max(1, Math.floor((dims.cols * current) / base + 1e-6)),
      rows: Math.max(1, Math.floor((dims.rows * current) / base + 1e-6))
    }
    entry.room = kept
  }
  const room = { cols: kept.cols, rows: kept.rows }
  reportViewport(terminalId, room)
  const fit = fitGrid({ cols: entry.term.cols, rows: entry.term.rows }, room, base)
  if (entry.term.options.fontSize !== fit.font) entry.term.options.fontSize = fit.font
  entry.panned = fit.pan.cols > 0 || fit.pan.rows > 0
  const wrapper = entry.persistentWrapper
  if (wrapper) wrapper.style.overflow = entry.panned ? 'auto' : 'hidden'
  measureCell(entry)
  if (wrapper) syncWindow(entry, wrapper)
}

function reportViewport(id: string, room: { cols: number; rows: number }): void {
  const s = sizingFor(id)
  if (s.viewport && s.viewport.cols === room.cols && s.viewport.rows === room.rows) return
  s.viewport = room
  window.api.terminalViewport?.({ id, cols: room.cols, rows: room.rows, pane: paneOf(id) })
}

function presenceOf(id: string): TerminalPresence {
  const entry = registry.get(id)
  if (!entry?.activeSlot || !entry.lastAppliedRect) return 'away'
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return 'away'
  return (sizing.get(id)?.activeUntil ?? 0) > Date.now() ? 'active' : 'watching'
}

/** Tells vornd whether a pane showing one of its sessions is in use, on screen, or hidden, when that changed. */
function reportPresence(id: string): void {
  const s = sizing.get(id)
  if (!s || !vorndStreams.has(id)) return
  const now = presenceOf(id)
  if (now === s.presence) return
  s.presence = now
  window.api.terminalPresence?.(id, now, paneOf(id))
}

/** Input or scrolling in a pane: in use for a few seconds from now. */
function markActive(id: string): void {
  const s = sizing.get(id)
  if (!s) return
  s.activeUntil = Date.now() + ACTIVE_MS
  if (s.activeTimer) clearTimeout(s.activeTimer)
  s.activeTimer = setTimeout(() => {
    s.activeTimer = null
    reportPresence(id)
  }, ACTIVE_MS + 50)
  reportPresence(id)
}

/** Where this pane stands toward the size of a session vornd holds; null for any other session. */
export interface TerminalSizing {
  /** The session's size is this pane's. */
  owner: boolean
  /** This pane locked it. */
  locked: boolean
}

export function getTerminalSizing(terminalId: string): TerminalSizing | null {
  const s = sizing.get(terminalId)
  if (!s || !vorndStreams.has(terminalId)) return null
  const pane = registry.get(terminalId)?.pane
  return { owner: s.client !== null && s.owner === `${s.client}:${pane}`, locked: s.locked }
}

/** Told when any pane's standing toward its session's size changes. */
export function onTerminalSizingChange(cb: () => void): () => void {
  sizingListeners.add(cb)
  return () => {
    sizingListeners.delete(cb)
  }
}

/** "Fit to this device": the session takes this pane's size. */
export function fitTerminalToDevice(terminalId: string): void {
  if (!vorndStreams.has(terminalId)) return
  window.api.takeTerminalSize?.(terminalId, paneOf(terminalId))
}

/**
 * Lock the session's size to this pane's, or release the lock. vornd refuses
 * a lock another client holds; the pane shows a lock only once vornd took it.
 * Answers whether it did.
 */
export async function setTerminalSizeLock(terminalId: string, locked: boolean): Promise<boolean> {
  if (!vorndStreams.has(terminalId) || !window.api.lockTerminalSize) return false
  const answer = await window.api.lockTerminalSize(terminalId, locked, paneOf(terminalId))
  const s = sizing.get(terminalId)
  if (!answer?.ok) {
    console.warn('[terminal] size lock refused', terminalId, answer?.error)
    return false
  }
  if (s) {
    s.locked = locked
    notifySizing()
  }
  return true
}

/** The first box is taken at once; a later one once it settles, so a drag is one refit and one SIGWINCH rather than one per frame. */
function fitWhenSettled(entry: TerminalEntry, terminalId: string): void {
  if (entry.lastSyncedCols === 0) {
    fitNow(entry, terminalId)
    return
  }
  if (entry.resizeTimer) clearTimeout(entry.resizeTimer)
  entry.resizeTimer = setTimeout(() => fitNow(entry, terminalId), RESIZE_SETTLE_MS)
  // While the hold runs, the window follows the cursor; see `receive`.
  overlaySync?.request()
  entry.resizeDeadline ??= setTimeout(() => fitNow(entry, terminalId), RESIZE_MAX_WAIT_MS)
}

/**
 * Whenever the overlay sync asks: the box follows the slot at once, the grid takes its size once it settles, and the pty hears only a changed size.
 *
 * True when the wrapper moved, was shown or hidden, or its window changed, which is how the sync knows things are still moving.
 */
export function syncTerminalOverlay(terminalId: string): boolean {
  const entry = registry.get(terminalId)
  const wrapper = entry?.persistentWrapper
  if (!entry || !wrapper) return false
  const slot = entry.activeSlot
  if (!slot) return hideWrapper(wrapper, entry)
  // The grid's box is what it is fitted to; with a fit element the slot is only the window onto it.
  const raw = (entry.fitElement ?? slot).getBoundingClientRect()
  const winRaw = entry.fitElement ? slot.getBoundingClientRect() : raw
  if (raw.width <= 0 || raw.height <= 0 || winRaw.height <= 0) return hideWrapper(wrapper, entry)
  const rect = {
    top: Math.round(raw.top),
    left: Math.round(raw.left),
    width: Math.round(raw.width),
    height: Math.round(raw.height)
  }
  const win = { top: Math.round(winRaw.top), height: Math.round(winRaw.height) }
  const last = entry.lastAppliedRect
  const same =
    last !== null &&
    last.top === rect.top &&
    last.left === rect.left &&
    last.width === rect.width &&
    last.height === rect.height
  if (!same) {
    const sizeChanged = last === null || last.width !== rect.width || last.height !== rect.height
    wrapper.style.left = `${rect.left}px`
    if (sizeChanged) {
      wrapper.style.width = `${rect.width}px`
      wrapper.style.height = `${rect.height}px`
    }
    wrapper.style.visibility = 'visible'
    wrapper.style.pointerEvents = 'auto'
    const shown = last === null
    entry.lastAppliedRect = rect
    applyWindow(entry, wrapper, rect, win)
    if (sizeChanged && entry.term.element) fitWhenSettled(entry, terminalId)
    if (shown) reportPresence(terminalId)
    return true
  }
  return applyWindow(entry, wrapper, rect, win)
}

export function onRegistryChange(cb: () => void): () => void {
  registryChangeListeners.add(cb)
  return () => {
    registryChangeListeners.delete(cb)
  }
}

export function getRegisteredTerminalIds(): string[] {
  if (!cachedTerminalIds) cachedTerminalIds = Array.from(registry.keys())
  return cachedTerminalIds
}

/**
 * Fit the terminal to its persistent wrapper and notify the pty of new size.
 *
 * Called for anything that changes the size of a *cell* as well as anything
 * that changes the size of the box. `syncTerminalOverlay` only fits when the
 * wrapper's rect moves, which is the right trigger for a resized card and the
 * wrong one for a renderer swap or a font arriving — both leave the box
 * identical and the column count stale, and the terminal then sat with a band
 * of unused width until something resized it.
 */
export function fitTerminal(terminalId: string, when: 'now' | 'settled' = 'now'): void {
  const entry = registry.get(terminalId)
  if (!entry) return
  if (when === 'now') fitNow(entry, terminalId)
  else fitWhenSettled(entry, terminalId)
}

/**
 * Focus the terminal (keyboard input).
 */
export function focusTerminal(terminalId: string): void {
  registry.get(terminalId)?.term.focus()
}

/** Whether this window is the one drawing that terminal, and so can speak for it. */
export function hasTerminal(terminalId: string): boolean {
  return registry.has(terminalId)
}

export function getTerminalSelection(terminalId: string): string {
  const entry = registry.get(terminalId)
  if (!entry || !entry.term.hasSelection()) return ''
  return entry.term.getSelection()
}

export function clearTerminalSelection(terminalId: string): void {
  registry.get(terminalId)?.term.clearSelection()
}

export function pasteToTerminal(terminalId: string, text: string): void {
  const entry = registry.get(terminalId)
  if (!entry) return
  if (entry.resizeTimer) fitNow(entry, terminalId)
  // xterm's paste sends the text and *then* clears its hidden textarea, which
  // only exists once the terminal is opened — so before that it throws after
  // sending, and a caller's following carriage return never runs.
  if (!entry.term.element) {
    window.api.writeTerminal(terminalId, text)
    return
  }
  entry.term.paste(text)
}

export function scrollToBottom(terminalId: string): void {
  const entry = registry.get(terminalId)
  if (!entry) return
  entry.term.scrollToBottom()
}

/** Buffer geometry for the command spine. Null when the terminal is gone. */
export function getTerminalBufferMetrics(terminalId: string): BufferMetrics | null {
  const entry = registry.get(terminalId)
  if (!entry) return null
  const buf = entry.term.buffer.active
  return {
    length: buf.length,
    viewportY: buf.viewportY,
    baseY: buf.baseY,
    rows: entry.term.rows,
    cursorLine: buf.baseY + buf.cursorY,
    isAlternate: buf.type === 'alternate'
  }
}

export function scrollTerminalToLine(terminalId: string, line: number): void {
  registry.get(terminalId)?.term.scrollToLine(line)
}

/**
 * Report which buffer row the pointer is over, so hovering anywhere in a
 * block highlights it — not just the narrow gutter beside it.
 *
 * Returns a disposer. Emits null when the pointer leaves the terminal.
 */
export function onTerminalRowHover(
  terminalId: string,
  cb: (line: number | null) => void
): () => void {
  const entry = registry.get(terminalId)
  const el = entry?.term.element
  if (!el) return () => {}

  // Cached across a hover: the geometry only changes on resize, and reading
  // it per mousemove forces a synchronous layout while the overlay loop is
  // writing styles every frame.
  let rect: DOMRect | null = null
  const handleEnter = (): void => {
    rect = el.getBoundingClientRect()
  }
  const handleMove = (e: MouseEvent): void => {
    if (!rect) rect = el.getBoundingClientRect()
    const rows = entry.term.rows
    if (rect.height <= 0 || rows <= 0) return
    const row = Math.floor(((e.clientY - rect.top) / rect.height) * rows)
    if (row < 0 || row >= rows) {
      cb(null)
      return
    }
    cb(entry.term.buffer.active.viewportY + row)
  }
  const handleLeave = (): void => {
    rect = null
    cb(null)
  }

  el.addEventListener('mouseenter', handleEnter)
  el.addEventListener('mousemove', handleMove)
  el.addEventListener('mouseleave', handleLeave)
  return () => {
    el.removeEventListener('mouseenter', handleEnter)
    el.removeEventListener('mousemove', handleMove)
    el.removeEventListener('mouseleave', handleLeave)
  }
}

/** Transient block highlight, one per terminal. */
const blockHighlights = new Map<string, { dispose(): void }>()

/**
 * Tint the rows of one block, so hovering its mark in the spine shows which
 * part of the session it covers. Pass null to clear.
 *
 * Transient by design: a decoration spanning N rows drifts if the terminal
 * reflows, which never happens inside a single hover.
 */
export function highlightTerminalBlock(
  terminalId: string,
  range: { startLine: number; endLine: number } | null
): void {
  blockHighlights.get(terminalId)?.dispose()
  blockHighlights.delete(terminalId)
  if (!range) return

  const entry = registry.get(terminalId)
  if (!entry) return
  const term = entry.term
  const buf = term.buffer.active
  if (buf.type === 'alternate') return

  // registerMarker takes an offset from the cursor, not an absolute row.
  const cursorLine = buf.baseY + buf.cursorY
  const marker = term.registerMarker(range.startLine - cursorLine)
  if (!marker) return

  const height = Math.max(1, range.endLine - range.startLine + 1)
  const decoration = term.registerDecoration({
    marker,
    width: term.cols,
    height,
    layer: 'bottom'
  })
  decoration?.onRender((el) => {
    if (el.dataset.vornBlockHl) return
    el.dataset.vornBlockHl = '1'
    el.style.width = '100%'
    el.style.background = 'rgba(255, 255, 255, 0.045)'
    el.style.pointerEvents = 'none'
  })

  blockHighlights.set(terminalId, {
    dispose: () => {
      decoration?.dispose()
      marker.dispose()
    }
  })
}

export function isAtBottom(terminalId: string): boolean {
  const entry = registry.get(terminalId)
  if (!entry) return true
  const buf = entry.term.buffer.active
  return buf.viewportY >= buf.baseY
}

export function onTerminalReady(terminalId: string, callback: () => void): () => void {
  if (registry.has(terminalId)) {
    callback()
    return () => {}
  }
  if (!readyCallbacks.has(terminalId)) readyCallbacks.set(terminalId, new Set())
  readyCallbacks.get(terminalId)!.add(callback)
  return () => {
    readyCallbacks.get(terminalId)?.delete(callback)
  }
}

/** Output as it lands, coalesced to a frame -- what the live region grows by. */
export function onTerminalWrite(
  terminalId: string,
  callback: () => void
): (() => void) | undefined {
  const entry = registry.get(terminalId)
  if (!entry) return undefined
  let timer: ReturnType<typeof setTimeout> | null = null
  const disposable = entry.term.onWriteParsed(() => {
    if (timer) return
    timer = setTimeout(() => {
      timer = null
      callback()
    }, 16)
  })
  return () => {
    disposable.dispose()
    if (timer) clearTimeout(timer)
  }
}

export function onTerminalScroll(
  terminalId: string,
  callback: () => void
): (() => void) | undefined {
  const entry = registry.get(terminalId)
  if (!entry) return undefined
  const scrollDisposable = entry.term.onScroll(callback)
  let writeTimer: ReturnType<typeof setTimeout> | null = null
  const writeDisposable = entry.term.onWriteParsed(() => {
    if (writeTimer) return
    writeTimer = setTimeout(() => {
      writeTimer = null
      callback()
    }, 300)
  })
  return () => {
    scrollDisposable.dispose()
    writeDisposable.dispose()
    if (writeTimer) clearTimeout(writeTimer)
  }
}

/**
 * Fully destroy a terminal (when killing an agent).
 */
export function destroyTerminal(terminalId: string): void {
  const entry = registry.get(terminalId)
  if (!entry) return
  // A seed still in flight would otherwise resolve and write into a terminal
  // that no longer exists.
  hydrating.delete(terminalId)
  vorndStreams.delete(terminalId)
  dropSizing(terminalId)
  clearHold(entry)
  entry._disposeCommandBlocks?.()
  entry._disposeCommandBlocks = null
  entry._disposeScrollAnchor?.()
  entry._disposeScrollAnchor = null
  // Dispose GPU addon first to avoid WebGL errors when the terminal
  // tears down the DOM element before the addon can clean up its GL context
  if (entry._gpuAddon) {
    try {
      entry._gpuAddon.dispose()
    } catch {
      // GL context may already be lost
    }
    entry._gpuAddon = null
  }
  entry.term.dispose()
  if (entry.persistentWrapper) {
    entry.persistentWrapper.remove()
    entry.persistentWrapper = null
  }
  entry.activeSlot = null
  registry.delete(terminalId)
  readyCallbacks.delete(terminalId)
  notifyRegistryChange()
}

/**
 * Update font size on all terminals and re-fit them.
 * Callers are responsible for clamping to MIN/MAX bounds.
 */
export function setAllTerminalsFontSize(fontSize: number): void {
  const effective = getEffectiveFontSize(fontSize)
  for (const [id, entry] of registry) {
    entry.term.options.fontSize = effective
    entry.userFont = effective
    // A pinch is many steps; the pty hears where it ends.
    fitTerminal(id, 'settled')
  }
}

/**
 * Get the current font size of the first mounted terminal (for UI display).
 */
export function getCurrentTerminalFontSize(): number {
  for (const entry of registry.values()) {
    return entry.term.options.fontSize ?? getEffectiveFontSize()
  }
  return getEffectiveFontSize()
}

/**
 * Re-fit all terminals that are currently overlaying an active slot.
 * Used when the virtual keyboard changes viewport geometry.
 */
export function fitAllTerminals(): void {
  for (const [id, entry] of registry) {
    if (entry.activeSlot) fitTerminal(id)
  }
}

/**
 * Keep one terminal's scroll anchor current. Returns a disposer.
 *
 * Settled rather than per event: xterm emits a scroll for every row that goes
 * past, and a `yarn build` would otherwise write to storage a few hundred times.
 */
function trackScrollAnchor(terminalId: string): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null
  const stop = onTerminalScroll(terminalId, () => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      const metrics = getTerminalBufferMetrics(terminalId)
      if (!metrics) return
      writeScrollAnchor(terminalId, chooseAnchor(getCommandBlocks(terminalId), metrics))
    }, ANCHOR_SETTLE_MS)
  })
  return () => {
    if (timer) clearTimeout(timer)
    timer = null
    stop?.()
  }
}

/** Put a freshly seeded terminal back where it was being read. */
function restoreScrollAnchor(terminalId: string): void {
  const line = resolveAnchor(getCommandBlocks(terminalId), readScrollAnchor(terminalId))
  if (line !== null) scrollTerminalToLine(terminalId, line)
}
