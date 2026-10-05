// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const { created } = vi.hoisted(() => ({
  created: [] as Array<{
    id: number
    write: ReturnType<typeof vi.fn>
    reset: ReturnType<typeof vi.fn>
    resize: ReturnType<typeof vi.fn>
    csi: Array<{ id: unknown; fn: (...a: unknown[]) => boolean }>
    options: { fontSize: number }
  }>
}))

vi.mock('@xterm/xterm', () => {
  let n = 0
  class MockTerminal {
    element: HTMLElement | null = null
    cols = 80
    rows = 24
    options = { fontSize: 13 }
    buffer = {
      active: { viewportY: 0, baseY: 0, type: 'normal' },
      onBufferChange: vi.fn().mockReturnValue({ dispose: vi.fn() })
    }
    csi: Array<{ id: unknown; fn: (...a: unknown[]) => boolean }> = []
    parser = {
      registerOscHandler: vi.fn().mockReturnValue({ dispose: vi.fn() }),
      registerDcsHandler: vi.fn().mockReturnValue({ dispose: vi.fn() }),
      registerCsiHandler: vi.fn((id: unknown, fn: (...a: unknown[]) => boolean) => {
        this.csi.push({ id, fn })
        return { dispose: vi.fn() }
      })
    }
    registerMarker = vi.fn()
    registerDecoration = vi.fn()
    loadAddon = vi.fn((addon: { activate?: (t: unknown) => void }) => addon.activate?.(this))
    onData = vi.fn()
    attachCustomKeyEventHandler = vi.fn()
    dispose = vi.fn()
    focus = vi.fn()
    // xterm.js runs a write's callback once what came before it is parsed.
    write = vi.fn((_data: unknown, done?: () => void) => done?.())
    reset = vi.fn()
    resize = vi.fn((cols: number, rows: number) => {
      this.cols = cols
      this.rows = rows
    })
    clearSelection = vi.fn()
    paste = vi.fn()
    scrollToBottom = vi.fn()
    scrollToLine = vi.fn()
    refresh = vi.fn()
    constructor() {
      created.push({
        id: n++,
        write: this.write,
        reset: this.reset,
        resize: this.resize,
        csi: this.csi,
        options: this.options
      })
    }
    open(el: HTMLElement): void {
      this.element = el
    }
    hasSelection(): boolean {
      return false
    }
    getSelection(): string {
      return ''
    }
    onScroll(): { dispose: () => void } {
      return { dispose: vi.fn() }
    }
    onWriteParsed(): { dispose: () => void } {
      return { dispose: vi.fn() }
    }
  }
  return { Terminal: MockTerminal }
})

vi.mock('@xterm/addon-fit', () => ({
  // The pane's box fits 80x24. Its cells are 8x16 px at 13 px and scale with
  // the font, as a monospace font's do.
  FitAddon: class {
    term: FitAddon['term'] = null
    activate(term: unknown): void {
      this.term = term as FitAddon['term']
    }
    fit = vi.fn(() => {
      if (!this.term?.element) return
      this.term.cols = 80
      this.term.rows = 24
    })
    proposeDimensions(): { cols: number; rows: number } | undefined {
      const el = this.term?.element as HTMLElement | null | undefined
      if (!el || !this.term) return undefined
      const scale = this.term.options.fontSize / 13
      return {
        cols: Math.floor((parseInt(el.style.width) || 0) / (8 * scale)),
        rows: Math.floor((parseInt(el.style.height) || 0) / (16 * scale))
      }
    }
  }
}))
type FitAddon = {
  term: { cols: number; rows: number; element: unknown; options: { fontSize: number } } | null
}
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

let frameListener: (bytes: Uint8Array) => void = () => {}
let resizedListener: (e: {
  id: string
  cols: number
  rows: number
  rseq: number
  owner?: string | null
  reason?: string | null
}) => void = () => {}
let reconnectedListener: () => void = () => {}
const attachTerminal = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    onTerminalData: () => () => {},
    onTerminalResync: () => () => {},
    onTerminalFrame: (cb: (bytes: Uint8Array) => void) => {
      frameListener = cb
      return () => {}
    },
    onTerminalResized: (cb: typeof resizedListener) => {
      resizedListener = cb
      return () => {}
    },
    onTerminalReconnected: (cb: () => void) => {
      reconnectedListener = cb
      return () => {}
    },
    attachTerminal,
    writeTerminal: vi.fn(),
    resizeTerminal: vi.fn(),
    terminalViewport: vi.fn(),
    terminalPresence: vi.fn(),
    takeTerminalSize: vi.fn(),
    lockTerminalSize: vi.fn(),
    openExternal: vi.fn()
  },
  writable: true
})

import {
  registerSlot,
  destroyTerminal,
  hydrateTerminal,
  fitTerminal,
  setHostRoot,
  syncTerminalOverlay,
  initGlobalDataListener,
  disposeGlobalDataListener,
  getTerminalSizing,
  fitTerminalToDevice,
  setTerminalSizeLock,
  getPersistentWrapper,
  unregisterSlot
} from '../src/renderer/lib/terminal-registry'
import { encodeTerminalFrameV2 } from '../packages/shared/src/terminal-frame'

/**
 * A pane showing a session vornd holds: its output arrives as version 2
 * frames that name their records, resizes arrive in order between them, and
 * after the connection comes back the pane attaches again from where its
 * screen ends, keeping that screen when vornd continues the stream. This is
 * the renderer's half of the reconnect rule; the web client and the phone run
 * the same registry over the web shim.
 */

const ID = 'held-by-vornd'

function slot(): HTMLDivElement {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({
      top: 0,
      left: 0,
      width: 400,
      height: 300,
      right: 400,
      bottom: 300,
      x: 0,
      y: 0,
      toJSON: () => ({})
    }) as DOMRect
  return el
}

const term = (): (typeof created)[number] => created[created.length - 1]!
const text = (d: unknown): string =>
  typeof d === 'string' ? d : new TextDecoder().decode(d as Uint8Array)
// The empty writes are the barriers resizes wait behind.
const writes = (): string[] =>
  term()
    .write.mock.calls.map((c) => text(c[0]))
    .filter((w) => w !== '')

/** A frame of records `first..=last`, its bytes starting at `offset`. */
function send(first: number, last: number, offset: number, data: string): void {
  frameListener(
    encodeTerminalFrameV2({
      id: ID,
      epoch: 0,
      firstRseq: first,
      lastRseq: last,
      startOffset: offset,
      data: new TextEncoder().encode(data)
    })
  )
}

/** vornd's answer to an attach with no cursor: a snapshot ending at `cursor`. */
function snapshot(nextRseq: number, nextOffset: number, data = 'SNAPSHOT'): unknown {
  return {
    data,
    seq: nextRseq - 1,
    live: true,
    cursor: { epoch: 0, nextRseq, nextOffset },
    continued: false,
    cols: 100,
    rows: 30,
    replies: 'vornd'
  }
}

beforeEach(() => {
  created.length = 0
  attachTerminal.mockReset()
  initGlobalDataListener()
})

afterEach(() => {
  destroyTerminal(ID)
  disposeGlobalDataListener()
})

describe('a session vornd holds', () => {
  it('draws the snapshot at the session size, then the frames after it', async () => {
    attachTerminal.mockResolvedValue(snapshot(8, 120))
    registerSlot(ID, slot())
    await hydrateTerminal(ID)
    send(8, 9, 120, 'after')

    expect(term().resize).toHaveBeenCalledWith(100, 30)
    expect(writes()).toEqual(['SNAPSHOT', 'after'])
  })

  it('holds frames that arrive during the attach, and drops those the snapshot has', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))
    registerSlot(ID, slot())
    const attaching = hydrateTerminal(ID)
    send(6, 7, 100, 'in the snapshot')
    resizedListener({ id: ID, cols: 90, rows: 20, rseq: 8 })
    send(9, 9, 120, 'after the resize')
    answer(snapshot(8, 120))
    await attaching

    expect(writes()).toEqual(['SNAPSHOT', 'after the resize'])
    // The resize at rseq 8 came before those bytes, and after the snapshot.
    const sizes = term().resize.mock.calls
    expect(sizes[sizes.length - 1]).toEqual([90, 20])
  })

  it('attaches again from where its screen ends, and keeps the screen when vornd continues', async () => {
    attachTerminal.mockResolvedValueOnce(snapshot(8, 120))
    registerSlot(ID, slot())
    await hydrateTerminal(ID)
    send(8, 10, 120, 'abc')
    resizedListener({ id: ID, cols: 80, rows: 24, rseq: 11 })

    attachTerminal.mockResolvedValueOnce({
      data: '',
      seq: 11,
      live: true,
      cursor: { epoch: 0, nextRseq: 12, nextOffset: 123 },
      continued: true,
      replies: 'vornd'
    })
    reconnectedListener()
    await vi.waitFor(() => expect(attachTerminal).toHaveBeenCalledTimes(2))
    await new Promise((r) => setTimeout(r, 0))
    send(12, 12, 123, 'continued')

    expect(attachTerminal.mock.calls[1]).toEqual([ID, { epoch: 0, nextRseq: 12, nextOffset: 123 }])
    expect(term().reset).not.toHaveBeenCalled()
    expect(writes()).toEqual(['SNAPSHOT', 'abc', 'continued'])
  })

  it('replaces the screen with the snapshot when vornd cannot continue', async () => {
    attachTerminal.mockResolvedValueOnce(snapshot(8, 120))
    registerSlot(ID, slot())
    await hydrateTerminal(ID)
    send(8, 8, 120, 'x')

    attachTerminal.mockResolvedValueOnce({
      ...(snapshot(50, 9000, 'NEWER') as object),
      resync: 'notRetained'
    })
    reconnectedListener()
    await vi.waitFor(() => expect(term().reset).toHaveBeenCalledTimes(1))

    expect(attachTerminal.mock.calls[1]).toEqual([ID, { epoch: 0, nextRseq: 9, nextOffset: 121 }])
    expect(writes()).toEqual(['SNAPSHOT', 'x', 'NEWER'])
  })

  it('leaves its device-attributes query to vornd, and answers one for any other session', async () => {
    attachTerminal.mockResolvedValue(snapshot(0, 0, ''))
    registerSlot(ID, slot())
    await hydrateTerminal(ID)
    const da1 = term().csi.find((h) => JSON.stringify(h.id) === JSON.stringify({ final: 'c' }))!
    expect(da1.fn([])).toBe(true)

    destroyTerminal(ID)
    attachTerminal.mockResolvedValue({ data: '', seq: 0, live: true })
    registerSlot(ID, slot())
    await hydrateTerminal(ID)
    const other = term().csi.find((h) => JSON.stringify(h.id) === JSON.stringify({ final: 'c' }))!
    expect(other.fn([])).toBe(false)
  })
})

describe('the size of a session vornd holds', () => {
  const api = window.api as unknown as Record<string, ReturnType<typeof vi.fn>>
  let host: HTMLDivElement

  /** A pane of `w`x`h` px showing the session at `cols`x`rows`. */
  async function show(cols: number, rows: number, w = 400, h = 300): Promise<HTMLDivElement> {
    host = document.createElement('div')
    document.body.appendChild(host)
    setHostRoot(host)
    attachTerminal.mockResolvedValue({
      ...(snapshot(8, 120) as object),
      cols,
      rows,
      client: 'ws:7'
    })
    const el = slot()
    el.getBoundingClientRect = () =>
      ({ top: 0, left: 0, width: w, height: h, right: w, bottom: h, x: 0, y: 0 }) as DOMRect
    registerSlot(ID, el)
    syncTerminalOverlay(ID)
    await hydrateTerminal(ID)
    fitTerminal(ID)
    return el
  }

  beforeEach(() => {
    for (const f of Object.values(api)) if (typeof f?.mockClear === 'function') f.mockClear()
  })

  afterEach(() => {
    setHostRoot(null)
    host?.remove()
  })

  it("never resizes the session: it reports this pane's viewport, and only when it changes", async () => {
    await show(80, 24)
    // Another client took the session to 100x30.
    resizedListener({ id: ID, cols: 100, rows: 30, rseq: 8, owner: 'ws:3', reason: 'input' })
    expect(term().resize).toHaveBeenLastCalledWith(100, 30)
    fitTerminal(ID)
    fitTerminal(ID)

    expect(window.api.resizeTerminal).not.toHaveBeenCalled()
    expect(api.terminalViewport.mock.calls).toEqual([[{ id: ID, cols: 50, rows: 18 }]])
  })

  it('draws a larger grid whole: the font scales down, to 9 px, and below that the pane pans', async () => {
    const opts = (): { fontSize: number } => term().options
    // 50x18 cells fit at 13 px; a 60x20 grid fits at 13 * 50/60 px.
    await show(60, 20)
    expect(opts().fontSize).toBeCloseTo((13 * 50) / 60, 5)
    expect(getPersistentWrapper(ID)!.style.overflow).toBe('hidden')
    // 120 columns would need 5.4 px: 9 px it is, and the rest is panned to.
    resizedListener({ id: ID, cols: 120, rows: 20, rseq: 8, owner: 'ws:3', reason: 'input' })
    expect(opts().fontSize).toBe(9)
    expect(getPersistentWrapper(ID)!.style.overflow).toBe('auto')
    // And a grid that fits is drawn at the user's font, with nothing to pan.
    resizedListener({ id: ID, cols: 40, rows: 10, rseq: 9, owner: 'ws:7', reason: 'input' })
    expect(opts().fontSize).toBe(13)
    expect(getPersistentWrapper(ID)!.style.overflow).toBe('hidden')
    // The viewport is still counted at the user's font throughout.
    expect(api.terminalViewport.mock.calls.every(([v]) => v.cols === 50 && v.rows === 18)).toBe(
      true
    )
  })

  it('says it is watching when shown, active when used, and away when hidden', async () => {
    const el = await show(80, 24)
    expect(api.terminalPresence.mock.calls).toEqual([[ID, 'watching']])
    getPersistentWrapper(ID)!.dispatchEvent(new Event('wheel'))
    expect(api.terminalPresence).toHaveBeenLastCalledWith(ID, 'active')
    unregisterSlot(ID, el)
    expect(api.terminalPresence).toHaveBeenLastCalledWith(ID, 'away')
  })

  it('knows when the size is its own, and asks for it or locks it on request', async () => {
    await show(80, 24)
    expect(getTerminalSizing(ID)).toEqual({ owner: false, locked: false })
    resizedListener({ id: ID, cols: 50, rows: 18, rseq: 8, owner: 'ws:7', reason: 'explicit' })
    expect(getTerminalSizing(ID)).toEqual({ owner: true, locked: false })
    fitTerminalToDevice(ID)
    expect(api.takeTerminalSize).toHaveBeenCalledWith(ID)
    setTerminalSizeLock(ID, true)
    expect(api.lockTerminalSize).toHaveBeenCalledWith(ID, true)
    expect(getTerminalSizing(ID)).toEqual({ owner: true, locked: true })
  })

  it('leaves a session vornd does not hold to the pane, as before', async () => {
    host = document.createElement('div')
    document.body.appendChild(host)
    setHostRoot(host)
    attachTerminal.mockResolvedValue({ data: '', seq: 0, live: true })
    registerSlot(ID, slot())
    syncTerminalOverlay(ID)
    await hydrateTerminal(ID)
    fitTerminal(ID)
    expect(getTerminalSizing(ID)).toBeNull()
    expect(api.terminalViewport).not.toHaveBeenCalled()
    expect(api.terminalPresence).not.toHaveBeenCalled()
  })
})
