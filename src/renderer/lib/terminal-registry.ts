import { Terminal, type ITerminalAddon } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import '@xterm/xterm/css/xterm.css'
import { attachCommandBlocks, jumpToCommand, markSeededFromServer } from './command-blocks'
import type { BufferMetrics } from './spine-layout'
import { TERMINAL_BACKGROUND } from '../../shared/surface'

interface TerminalEntry {
  term: Terminal
  fitAddon: FitAddon
  persistentWrapper: HTMLDivElement | null
  activeSlot: HTMLElement | null
  lastAppliedRect: { top: number; left: number; width: number; height: number } | null
  lastSyncedCols: number
  lastSyncedRows: number
  _loadRenderer?: (() => void) | null
  _gpuAddon?: { dispose(): void } | null
  _disposeCommandBlocks?: (() => void) | null
  /** Whether this terminal has already been seeded from the server. */
  _hydrated?: boolean
}

/** data attribute on the persistent wrapper, read by TerminalHost for event delegation. */
export const TERMINAL_ID_ATTR = 'data-terminal-id'

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
 * The server's own screen model has been guarded against this since it was
 * written (`pty-manager-screen.test.ts`); the client had the same hole and no
 * seeded screen to fall into it until this branch.
 */
const seeding = new Set<string>()
const readyCallbacks = new Map<string, Set<() => void>>()

// --- Write batching: single global listener + requestAnimationFrame ---
interface Chunk {
  data: string
  /** Which flush of that session this came from. See `PtyManager.flushSeq`. */
  seq: number
}

const pendingWrites = new Map<string, Chunk[]>()
let rafId: number | null = null

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

function scheduleFlush(): void {
  if (rafId !== null) return
  rafId = requestAnimationFrame(flushWrites)
}

function flushWrites(): void {
  rafId = null
  for (const [id, chunks] of pendingWrites) {
    const seeding = hydrating.get(id)
    if (seeding) {
      for (const chunk of chunks) seeding.held.push(chunk)
      continue
    }
    const data = chunks.length === 1 ? chunks[0].data : chunks.map((c) => c.data).join('')
    const entry = registry.get(id)
    if (entry) entry.term.write(data)
    // Only when it was actually written. A handler that fires for a terminal
    // nothing drew into is reporting something nobody saw -- and the one handler
    // that exists rings a bell.
    if (entry) statusHandlers.get(id)?.(data)
  }
  pendingWrites.clear()
}

let removeGlobalDataListener: (() => void) | null = null

export function initGlobalDataListener(): void {
  if (removeGlobalDataListener) return
  removeGlobalDataListener = window.api.onTerminalData(({ id, data, seq }) => {
    const existing = pendingWrites.get(id)
    if (existing) {
      existing.push({ data, seq })
    } else {
      pendingWrites.set(id, [{ data, seq }])
    }
    scheduleFlush()
  })
}

export function disposeGlobalDataListener(): void {
  removeGlobalDataListener?.()
  removeGlobalDataListener = null
  if (rafId !== null) {
    cancelAnimationFrame(rafId)
    rafId = null
  }
  pendingWrites.clear()
  hydrating.clear()
  seeding.clear()
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
export function hydrateTerminal(terminalId: string): Promise<void> {
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

  /** Everything held, in order, as one write. xterm queues a task per call. */
  const flushHeld = (above = -1): void => {
    const kept = state.held.filter((chunk) => chunk.seq > above)
    if (!kept.length) return
    const data = kept.map((chunk) => chunk.data).join('')
    entry.term.write(data)
    // Live, unlike the seed: a bell that rang while the seed was in flight rang
    // just now, and holding it back for a round trip would drop it entirely.
    statusHandlers.get(terminalId)?.(data)
  }

  state.done = (async () => {
    try {
      const { data, seq, live } = await window.api.attachTerminal(terminalId)
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
      // The one moment a pane learns the truth about its session. A window
      // opened onto a terminal that died while it was closed has no start-up
      // reconciliation to tell it -- this is where it finds out.
      if (live === false) reportNotLive?.(terminalId)
    } catch (err) {
      console.error('[terminal] could not attach', terminalId, err)
      // Held chunks are still the truth about what happened; let them through
      // rather than losing them to a failed seed, and allow another try.
      flushHeld()
      entry._hydrated = false
    } finally {
      hydrating.delete(terminalId)
    }
  })()
  return state.done
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

export function setNotLiveReporter(fn: NotLiveReporter | null): void {
  reportNotLive = fn
}

// --- Status detection handler registry ---
type StatusHandler = (data: string) => void
const statusHandlers = new Map<string, StatusHandler>()

export function registerStatusHandler(terminalId: string, handler: StatusHandler): () => void {
  statusHandlers.set(terminalId, handler)
  return () => {
    statusHandlers.delete(terminalId)
  }
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

  // Forward keystrokes to pty
  term.onData((data) => {
    if (seeding.has(terminalId)) return
    window.api.writeTerminal(terminalId, data)
  })

  const disposeCommandBlocks = attachCommandBlocks(terminalId, term)

  const entry: TerminalEntry = {
    term,
    fitAddon,
    persistentWrapper: null,
    activeSlot: null,
    lastAppliedRect: null,
    lastSyncedCols: 0,
    lastSyncedRows: 0
  }

  entry._loadRenderer = loadRenderer
  entry._disposeCommandBlocks = disposeCommandBlocks

  registry.set(terminalId, entry)

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
  wrapper.style.left = '0'
  wrapper.style.width = '0'
  wrapper.style.height = '0'
  wrapper.style.visibility = 'hidden'
  wrapper.style.pointerEvents = 'none'
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

/**
 * Register a slot element for a terminal. The wrapper is created (lazily)
 * and will track this slot's bounding rect via syncTerminalOverlay.
 * Last-registered slot wins if multiple slots register for the same id.
 */
export function registerSlot(terminalId: string, slotEl: HTMLElement): void {
  let entry = registry.get(terminalId)
  if (!entry) entry = createTerminalEntry(terminalId)
  entry.activeSlot = slotEl
  ensurePersistentWrapper(entry, terminalId)
  openIntoPersistentWrapper(entry, terminalId)
  syncTerminalOverlay(terminalId)
}

/**
 * Unregister a slot. No-op if the current active slot is not this element
 * (protects against out-of-order unmounts during rapid view swaps).
 */
export function unregisterSlot(terminalId: string, slotEl: HTMLElement): void {
  const entry = registry.get(terminalId)
  if (!entry || entry.activeSlot !== slotEl) return
  entry.activeSlot = null
  syncTerminalOverlay(terminalId)
}

export function getPersistentWrapper(terminalId: string): HTMLDivElement | null {
  return registry.get(terminalId)?.persistentWrapper ?? null
}

function hideWrapper(wrapper: HTMLDivElement, entry: TerminalEntry): void {
  if (wrapper.style.visibility !== 'hidden') {
    wrapper.style.visibility = 'hidden'
    wrapper.style.pointerEvents = 'none'
  }
  entry.lastAppliedRect = null
}

/**
 * Position the persistent wrapper to overlay the active slot. Called every
 * frame by TerminalHost, so the function is aggressively guarded:
 * rect is rounded to integer pixels so subpixel spring jitter doesn't
 * trigger a fit+IPC each frame, and pty resize IPC only fires when the
 * integer cols/rows actually change. visibility is used (not display:none)
 * because xterm needs nonzero layout metrics to fit correctly.
 */
export function syncTerminalOverlay(terminalId: string): void {
  const entry = registry.get(terminalId)
  const wrapper = entry?.persistentWrapper
  if (!entry || !wrapper) return
  const slot = entry.activeSlot
  if (!slot) {
    hideWrapper(wrapper, entry)
    return
  }
  const raw = slot.getBoundingClientRect()
  if (raw.width <= 0 || raw.height <= 0) {
    hideWrapper(wrapper, entry)
    return
  }
  const rect = {
    top: Math.round(raw.top),
    left: Math.round(raw.left),
    width: Math.round(raw.width),
    height: Math.round(raw.height)
  }
  const last = entry.lastAppliedRect
  if (
    last !== null &&
    last.top === rect.top &&
    last.left === rect.left &&
    last.width === rect.width &&
    last.height === rect.height
  ) {
    return
  }
  // Size-changed matters for fit (cols/rows depend on width/height); position-
  // only changes (Framer Motion springs move cards via translate) just need a
  // style update and skip the layout-reading fitAddon.fit() call.
  const sizeChanged = last === null || last.width !== rect.width || last.height !== rect.height
  wrapper.style.top = `${rect.top}px`
  wrapper.style.left = `${rect.left}px`
  if (sizeChanged) {
    wrapper.style.width = `${rect.width}px`
    wrapper.style.height = `${rect.height}px`
  }
  wrapper.style.visibility = 'visible'
  wrapper.style.pointerEvents = 'auto'
  entry.lastAppliedRect = rect
  if (!sizeChanged || !entry.term.element) return
  try {
    entry.fitAddon.fit()
  } catch {
    return
  }
  const { cols, rows } = entry.term
  if (cols !== entry.lastSyncedCols || rows !== entry.lastSyncedRows) {
    entry.lastSyncedCols = cols
    entry.lastSyncedRows = rows
    window.api.resizeTerminal({ id: terminalId, cols, rows })
  }
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
export function fitTerminal(terminalId: string): void {
  const entry = registry.get(terminalId)
  if (!entry || !entry.term.element) return
  try {
    entry.fitAddon.fit()
  } catch {
    return
  }
  const { cols, rows } = entry.term
  if (cols !== entry.lastSyncedCols || rows !== entry.lastSyncedRows) {
    entry.lastSyncedCols = cols
    entry.lastSyncedRows = rows
    window.api.resizeTerminal({ id: terminalId, cols, rows })
  }
}

/**
 * Focus the terminal (keyboard input).
 */
export function focusTerminal(terminalId: string): void {
  registry.get(terminalId)?.term.focus()
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
  // Flush any pending batched writes before destroying. Joined by their data:
  // these became `{ data, seq }` when attaching needed a way to tell what a seed
  // already contained, and this line kept joining the objects -- which type-
  // checks, and writes `[object Object]` instead of the output it exists to save.
  const chunks = pendingWrites.get(terminalId)
  if (chunks) {
    entry.term.write(chunks.map((chunk) => chunk.data).join(''))
    pendingWrites.delete(terminalId)
  }
  // A seed still in flight would otherwise resolve and write into a terminal
  // that no longer exists.
  hydrating.delete(terminalId)
  statusHandlers.delete(terminalId)
  entry._disposeCommandBlocks?.()
  entry._disposeCommandBlocks = null
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
    fitTerminal(id)
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
