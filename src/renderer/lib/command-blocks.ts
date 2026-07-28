import type { IMarker, Terminal } from '@xterm/xterm'

/**
 * Command blocks: structured command boundaries inside the raw terminal.
 *
 * The shell integration shim emits OSC 133 sequences (A = prompt start,
 * C = execution start, D;code = finished) plus a private OSC 5522 carrying
 * the base64-encoded command text. This module turns those into markers on
 * the existing xterm buffer — the terminal rendering itself is never
 * touched.
 *
 * Nothing is drawn into the buffer. The blocks are consumed by the command
 * spine, a gutter rendered beside the terminal, which subscribes via
 * onCommandBlocksChange and reads positions from the markers. Markers also
 * serve as jump targets for Cmd/Ctrl+Up/Down navigation.
 */

/** Minimal marker surface — lets tests drive the tracker without xterm. */
export interface MarkerLike {
  readonly line: number
  readonly isDisposed: boolean
  dispose(): void
  onDispose(cb: () => void): void
}

export interface CommandBlock {
  command: string | null
  exitCode: number
  durationMs: number
  /** Directory the command ran in, as reported at the preceding prompt. */
  cwd: string | null
  /** Buffer rows the command's output occupied. Drives routine classification. */
  outputLines: number
  marker: MarkerLike
}

export interface TrackerHost {
  registerMarker(): MarkerLike | undefined
  /** Called when a command finishes. */
  onBlockFinished(block: CommandBlock): void
  /** Called when scrollback trimming disposes a block's marker. */
  onBlocksPruned?(): void
  /** Called when the shell reports its working directory at each prompt. */
  onCwdChanged?(cwd: string): void
  isAlternateBuffer(): boolean
  now(): number
  /** Absolute buffer row of the cursor, used to measure output volume. */
  currentLine?(): number
}

const MAX_BLOCKS = 200

export type ShellInputState = 'prompt' | 'running' | 'unknown'

export class CommandBlockTracker {
  readonly blocks: CommandBlock[] = []
  private promptMarker: MarkerLike | null = null
  private runningMarker: MarkerLike | null = null
  private runningCommand: string | null = null
  private runningSince = 0
  private runningStartLine = 0
  private runningCwd: string | null = null
  private lastCwd: string | null = null
  private pendingCommand: string | null = null
  private sawPrompt = false

  constructor(private host: TrackerHost) {}

  /**
   * Whether the shell is waiting at its prompt ('prompt'), executing a
   * command or showing a full-screen app ('running'), or has never reported
   * boundaries ('unknown' — no shell integration; degrade to raw input).
   */
  inputState(): ShellInputState {
    if (this.host.isAlternateBuffer()) return 'running'
    if (!this.sawPrompt) return 'unknown'
    return this.promptMarker && !this.promptMarker.isDisposed ? 'prompt' : 'running'
  }

  /** OSC 133 payload: "A" | "C" | "D;<code>" (B is unused by the shim). */
  handleSequence(payload: string): void {
    if (this.host.isAlternateBuffer()) return
    const kind = payload[0]
    if (kind === 'A') {
      // A new prompt. An unconsumed previous prompt marker means the user
      // pressed Enter on an empty line — drop it, it isn't a block.
      if (this.promptMarker && !this.promptMarker.isDisposed) this.promptMarker.dispose()
      this.promptMarker = this.host.registerMarker() ?? null
      this.sawPrompt = true
      return
    }
    if (kind === 'C') {
      this.runningMarker = this.promptMarker
      this.promptMarker = null
      this.runningCommand = this.pendingCommand
      this.pendingCommand = null
      this.runningSince = this.host.now()
      this.runningStartLine = this.host.currentLine?.() ?? 0
      // The prompt that preceded this command reported the directory it will
      // run in.
      this.runningCwd = this.lastCwd
      return
    }
    if (kind === 'D') {
      if (!this.runningMarker || this.runningMarker.isDisposed) {
        this.resetRunning()
        return
      }
      const exitCode = Number.parseInt(payload.slice(2), 10)
      const endLine = this.host.currentLine?.() ?? 0
      const block: CommandBlock = {
        command: this.runningCommand,
        exitCode: Number.isNaN(exitCode) ? 0 : exitCode,
        durationMs: Math.max(0, this.host.now() - this.runningSince),
        cwd: this.runningCwd,
        outputLines: Math.max(0, endLine - this.runningStartLine),
        marker: this.runningMarker
      }
      this.resetRunning()
      this.blocks.push(block)
      block.marker.onDispose(() => {
        const i = this.blocks.indexOf(block)
        if (i !== -1) this.blocks.splice(i, 1)
        this.host.onBlocksPruned?.()
      })
      while (this.blocks.length > MAX_BLOCKS) {
        // dispose triggers the onDispose pruning above
        this.blocks[0].marker.dispose()
      }
      this.host.onBlockFinished(block)
    }
  }

  /** OSC 5522 payload: "cmd;<base64 of the command text>" or "cwd;<path>". */
  handleCommandText(payload: string): void {
    if (this.host.isAlternateBuffer()) return
    if (payload.startsWith('cwd;')) {
      const cwd = payload.slice(4)
      if (cwd.startsWith('/')) {
        this.lastCwd = cwd
        this.host.onCwdChanged?.(cwd)
      }
      return
    }
    if (!payload.startsWith('cmd;')) return
    try {
      const bytes = atob(payload.slice(4))
      this.pendingCommand = new TextDecoder().decode(Uint8Array.from(bytes, (c) => c.charCodeAt(0)))
    } catch {
      this.pendingCommand = null
    }
  }

  /**
   * The command currently executing, if any. The spine draws this as a
   * running mark; it has no exit code or duration yet.
   */
  runningBlock(): { command: string | null; since: number; marker: MarkerLike } | null {
    if (!this.runningMarker || this.runningMarker.isDisposed) return null
    return { command: this.runningCommand, since: this.runningSince, marker: this.runningMarker }
  }

  /** Buffer lines that act as jump targets, sorted ascending. */
  jumpLines(): number[] {
    const lines: number[] = []
    for (const b of this.blocks) {
      if (!b.marker.isDisposed) lines.push(b.marker.line)
    }
    if (this.runningMarker && !this.runningMarker.isDisposed) lines.push(this.runningMarker.line)
    if (this.promptMarker && !this.promptMarker.isDisposed) lines.push(this.promptMarker.line)
    return lines.sort((a, b) => a - b)
  }

  private resetRunning(): void {
    this.runningMarker = null
    this.runningCommand = null
    this.runningSince = 0
    this.runningStartLine = 0
    this.runningCwd = null
  }
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(1, Math.round(ms / 100)) / 10}s`
  const s = ms / 1000
  if (s < 10) return `${s.toFixed(1)}s`
  if (s < 60) return `${Math.round(s)}s`
  const m = Math.floor(s / 60)
  return `${m}m ${Math.round(s - m * 60)}s`
}

// --- xterm wiring ---

const trackers = new Map<string, CommandBlockTracker>()

// Shell-reported working directory per terminal. The store wiring (App
// startup) registers a reporter so session state can follow `cd`.
let cwdReporter: ((terminalId: string, cwd: string) => void) | null = null

export function setCwdReporter(reporter: (terminalId: string, cwd: string) => void): void {
  cwdReporter = reporter
}

const CHIP_FAIL = '#f87171'

/**
 * The tail of the path, so it reads as a hint rather than a full location.
 * Two segments is enough to tell `dev/vorn` from `vorn/packages`.
 */
export function shortenCwd(cwd: string | null): string | null {
  if (!cwd) return null
  const parts = cwd.split('/').filter(Boolean)
  if (parts.length === 0) return '/'
  if (parts.length <= 2) return `/${parts.join('/')}`
  return parts.slice(-2).join('/')
}

/**
 * How long the command took, right-aligned on its own row.
 *
 * The command's heading role is carried by weight (the shell renders it bold)
 * and by the dim directory line above it, not by a background band — a band
 * across the full width reads as a selection, not a heading.
 */
function drawCommandMeta(term: Terminal, marker: IMarker, block: CommandBlock): void {
  if (marker.isDisposed) return
  const ok = block.exitCode === 0
  const parts: string[] = []
  const dir = shortenCwd(block.cwd)
  if (dir) parts.push(dir)
  if (!ok) parts.push(`exit ${block.exitCode}`)
  parts.push(formatDuration(block.durationMs))
  const text = parts.join(' · ')
  // Spans the row and is pushed right with flexbox rather than relying on the
  // decoration's own right anchor, which lands the element at column 0 — on
  // top of the command it is supposed to annotate.
  const chip = term.registerDecoration({ marker, width: term.cols })
  chip?.onRender((el) => {
    if (el.dataset.vornMeta) return
    el.dataset.vornMeta = '1'
    el.style.width = '100%'
    el.style.position = 'relative'
    el.style.pointerEvents = 'none'
    // Absolutely positioned against the full-width row rather than laid out
    // by flexbox on the decoration element itself, which leaves the text at
    // column 0 — on top of the command it annotates.
    const label = document.createElement('span')
    label.textContent = text
    label.style.position = 'absolute'
    label.style.right = '0'
    label.style.top = '0'
    label.style.fontSize = '10px'
    label.style.whiteSpace = 'nowrap'
    label.style.color = ok ? 'rgba(255,255,255,0.30)' : CHIP_FAIL
    el.appendChild(label)
  })
}

/**
 * Close a finished block with a full-width rule.
 *
 * Drawn under the last row of output rather than above the next command: a
 * rule below reads as the end of what you were looking at, whereas one above
 * belongs to whatever comes next and leaves the previous block open. The
 * blank line the shell prints then sits below the rule, as separation between
 * blocks rather than an unexplained gap inside one.
 */
function drawBlockEnd(term: Terminal): void {
  const marker = term.registerMarker(0)
  if (!marker) return
  const rule = term.registerDecoration({ marker, width: term.cols, layer: 'bottom' })
  rule?.onRender((el) => {
    if (el.dataset.vornBlockEnd) return
    el.dataset.vornBlockEnd = '1'
    el.style.width = '100%'
    el.style.pointerEvents = 'none'
    // Centred in the blank row the shell prints, rather than drawn on its top
    // edge: that splits the row into padding above and below, so the boundary
    // is not pinned against the last line of output.
    el.style.display = 'flex'
    el.style.alignItems = 'center'
    const line = document.createElement('div')
    line.style.width = '100%'
    line.style.borderTop = '1px solid rgba(255, 255, 255, 0.09)'
    el.appendChild(line)
  })
}

// Change notification. The spine re-reads getCommandBlocks() on each emit
// rather than receiving a payload, so listeners never hold stale markers.
const blockListeners = new Map<string, Set<() => void>>()

export function onCommandBlocksChange(terminalId: string, cb: () => void): () => void {
  let set = blockListeners.get(terminalId)
  if (!set) {
    set = new Set()
    blockListeners.set(terminalId, set)
  }
  set.add(cb)
  return () => {
    const current = blockListeners.get(terminalId)
    current?.delete(cb)
    if (current && current.size === 0) blockListeners.delete(terminalId)
  }
}

function emitBlocksChanged(terminalId: string): void {
  const set = blockListeners.get(terminalId)
  if (!set) return
  for (const cb of set) cb()
}

/**
 * Attach OSC handlers to a terminal. Returns a dispose function.
 */
export function attachCommandBlocks(terminalId: string, term: Terminal): () => void {
  const tracker = new CommandBlockTracker({
    registerMarker: () => term.registerMarker(0) ?? undefined,
    onBlockFinished: (block) => {
      // Fired from the OSC handler, so the cursor is still sitting just past
      // the command's last line of output — exactly where the rule goes.
      drawCommandMeta(term, block.marker as IMarker, block)
      drawBlockEnd(term)
      emitBlocksChanged(terminalId)
    },
    onBlocksPruned: () => emitBlocksChanged(terminalId),
    onCwdChanged: (cwd) => cwdReporter?.(terminalId, cwd),
    isAlternateBuffer: () => term.buffer.active.type === 'alternate',
    now: () => Date.now(),
    currentLine: () => term.buffer.active.baseY + term.buffer.active.cursorY
  })
  trackers.set(terminalId, tracker)

  // While the shell waits at its prompt the intent bar owns input, so the
  // terminal's own cursor is hidden (painted in the background color) to
  // avoid two competing cursors. It returns as soon as a command runs.
  const defaultCursor = term.options.theme?.cursor
  const applyCursorVisibility = (): void => {
    const theme = term.options.theme ?? {}
    const background = theme.background ?? '#141416'
    const hide = tracker.inputState() === 'prompt'
    const cursor = hide ? background : defaultCursor
    if (theme.cursor !== cursor) {
      term.options.theme = { ...theme, cursor, cursorAccent: hide ? background : undefined }
    }
  }

  const d1 = term.parser.registerOscHandler(133, (data) => {
    tracker.handleSequence(data)
    applyCursorVisibility()
    // Emit on A, C and D so the spine shows the running mark as soon as a
    // command starts, not only once it finishes.
    emitBlocksChanged(terminalId)
    return true
  })
  const d2 = term.parser.registerOscHandler(5522, (data) => {
    tracker.handleCommandText(data)
    return true
  })

  return () => {
    d1.dispose()
    d2.dispose()
    trackers.delete(terminalId)
    blockListeners.delete(terminalId)
  }
}

export function getCommandBlocks(terminalId: string): CommandBlock[] {
  return trackers.get(terminalId)?.blocks ?? []
}

export function getRunningBlock(
  terminalId: string
): { command: string | null; since: number; marker: MarkerLike } | null {
  return trackers.get(terminalId)?.runningBlock() ?? null
}

/** Input routing signal for the terminal (see CommandBlockTracker.inputState). */
export function getShellInputState(terminalId: string): ShellInputState {
  return trackers.get(terminalId)?.inputState() ?? 'unknown'
}

/**
 * Scroll the viewport to the previous/next command boundary.
 */
export function jumpToCommand(terminalId: string, term: Terminal, direction: -1 | 1): void {
  const tracker = trackers.get(terminalId)
  if (!tracker || term.buffer.active.type === 'alternate') return
  const lines = tracker.jumpLines()
  if (lines.length === 0) return
  const current = term.buffer.active.viewportY
  const target =
    direction < 0 ? [...lines].reverse().find((l) => l < current) : lines.find((l) => l > current)
  if (target !== undefined) term.scrollToLine(target)
}
