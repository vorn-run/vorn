/**
 * The renderer's terminal path, in a page of its own.
 *
 * This is the real `terminal-registry.ts` -- the same xterm options, the same
 * WebGL addon, the same `syncTerminalOverlay` -- kept on its slots by the same
 * `startTerminalOverlaySync` `TerminalHost.tsx` starts, with `window.api`
 * stubbed to a feed. The rest
 * of the app (React, the stores, the panes) is left out on purpose: none of the
 * renderer hotspots in the plan are in it except Shiki, and this is the number
 * WP7 is judged on at 1, 8 and 32 terminals.
 */
import type { TerminalData } from '@vornrun/shared/protocol'
import {
  initGlobalDataListener,
  registerSlot,
  setHostRoot,
  startTerminalOverlaySync
} from '../../src/renderer/lib/terminal-registry'

type Listener = (d: { id: string; data: string; seq: number }) => void
let listener: Listener | null = null

;(window as unknown as { api: unknown }).api = {
  onTerminalData(cb: Listener) {
    listener = cb
    return () => (listener = null)
  },
  writeTerminal() {},
  resizeTerminal() {},
  openExternal() {}
}

export interface RendererRun {
  terminals: number
  /** How many of them are on screen; the rest stream into a view that is not shown, as in another tab. */
  shown: number
  frames: number
  frameMeanMs: number
  frameP95Ms: number
  frameP99Ms: number
  /** Share of frames that took longer than 1.5 vsync intervals. */
  droppedPct: number
  longTaskMs: number
  bytesWritten: number
  webglCanvases: number
  gpu: string
}

function percentile(values: number[], p: number): number {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))]
}

function gpuString(): string {
  const gl = document.createElement('canvas').getContext('webgl2')
  if (!gl) return 'no webgl2'
  const ext = gl.getExtension('WEBGL_debug_renderer_info')
  return ext
    ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL))
    : String(gl.getParameter(gl.RENDERER))
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

async function run(opts: {
  terminals: number
  /** Defaults to all of them. */
  shown?: number
  chunks: string[]
  bytesPerSecondEach: number
  durationMs: number
}): Promise<RendererRun> {
  const grid = document.getElementById('grid')!
  const host = document.getElementById('host')!
  const shown = Math.min(opts.shown ?? opts.terminals, opts.terminals)
  const cols = Math.ceil(Math.sqrt(shown))
  grid.style.gridTemplateColumns = `repeat(${cols}, 1fr)`
  grid.style.gridTemplateRows = `repeat(${Math.ceil(shown / cols)}, 1fr)`

  setHostRoot(host)
  initGlobalDataListener()
  const ids: string[] = []
  for (let i = 0; i < opts.terminals; i++) {
    const slot = document.createElement('div')
    // A slot that is not shown has no box, as a pane in a hidden view does.
    if (i >= shown) slot.style.display = 'none'
    grid.appendChild(slot)
    const id = `t${i}`
    ids.push(id)
    registerSlot(id, slot)
  }

  // As TerminalHost.tsx does.
  const stopSync = startTerminalOverlaySync(host)

  // The WebGL addon loads through a dynamic import; let it attach before measuring.
  await sleep(1000)

  let longTaskMs = 0
  const observer = new PerformanceObserver((list) => {
    for (const e of list.getEntries()) longTaskMs += e.duration
  })
  observer.observe({ entryTypes: ['longtask'] })

  // The server flushes every 8 ms; deliver each terminal's share on that beat.
  const perFlush = (opts.bytesPerSecondEach * 8) / 1000
  const cursors = ids.map((_, i) => Math.floor((i * opts.chunks.length) / ids.length))
  const seqs = ids.map(() => 0)
  // The budget is in UTF-8 bytes, so each chunk is charged its encoded size.
  const encoder = new TextEncoder()
  const sizes = opts.chunks.map((c) => encoder.encode(c).length)
  let bytesWritten = 0
  const feed = setInterval(() => {
    for (let i = 0; i < ids.length; i++) {
      let out = ''
      let size = 0
      while (size < perFlush) {
        out += opts.chunks[cursors[i]]
        size += sizes[cursors[i]]
        cursors[i] = (cursors[i] + 1) % opts.chunks.length
      }
      bytesWritten += size
      listener?.({ id: ids[i], data: out, seq: ++seqs[i] } as TerminalData & { data: string })
    }
  }, 8)

  const intervals: number[] = []
  await new Promise<void>((resolve) => {
    let last = 0
    const started = performance.now()
    requestAnimationFrame(function frame(t: number): void {
      if (last) intervals.push(t - last)
      last = t
      if (t - started >= opts.durationMs) resolve()
      else requestAnimationFrame(frame)
    })
  })
  clearInterval(feed)
  observer.disconnect()
  stopSync()

  const vsync = percentile(intervals, 10)
  return {
    terminals: opts.terminals,
    shown,
    frames: intervals.length,
    frameMeanMs: intervals.reduce((a, b) => a + b, 0) / intervals.length,
    frameP95Ms: percentile(intervals, 95),
    frameP99Ms: percentile(intervals, 99),
    droppedPct: (100 * intervals.filter((d) => d > vsync * 1.5).length) / intervals.length,
    longTaskMs,
    bytesWritten,
    webglCanvases: host.querySelectorAll('canvas').length,
    gpu: gpuString()
  }
}

;(window as unknown as { __bench: typeof run }).__bench = run
;(window as unknown as { __ready: boolean }).__ready = true
