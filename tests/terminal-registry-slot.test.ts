// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// The fit mock sizes the grid from the wrapper's box, 8x16 px cells, so a moved rect moves cols/rows.
const made = vi.hoisted(() => ({
  terms: [] as Array<{ onData: ReturnType<typeof vi.fn> }>,
  fits: [] as Array<ReturnType<typeof vi.fn>>
}))

vi.mock('@xterm/xterm', () => {
  class MockTerminal {
    element: HTMLElement | null = null
    cols = 80
    rows = 24
    options = { fontSize: 13 }
    buffer = {
      active: { viewportY: 0, baseY: 0, type: 'normal' },
      onBufferChange: vi.fn().mockReturnValue({ dispose: vi.fn() })
    }
    parser = {
      registerOscHandler: vi.fn().mockReturnValue({ dispose: vi.fn() }),
      registerCsiHandler: vi.fn().mockReturnValue({ dispose: vi.fn() })
    }
    registerMarker = vi.fn()
    registerDecoration = vi.fn()
    loadAddon = vi.fn((addon: { activate?: (t: unknown) => void }) => addon.activate?.(this))
    onData = vi.fn()
    attachCustomKeyEventHandler = vi.fn()
    dispose = vi.fn()
    focus = vi.fn()
    write = vi.fn()
    clearSelection = vi.fn()
    paste = vi.fn()
    scrollToBottom = vi.fn()
    scrollToLine = vi.fn()
    refresh = vi.fn()
    constructor() {
      made.terms.push(this)
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

vi.mock('@xterm/addon-fit', () => {
  class MockFitAddon {
    term: { element: HTMLElement | null; cols: number; rows: number } | null = null
    activate(term: unknown): void {
      this.term = term as MockFitAddon['term']
    }
    proposeDimensions(): { cols: number; rows: number } | undefined {
      const el = this.term?.element
      if (!el) return undefined
      return {
        cols: Math.max(2, Math.floor((parseInt(el.style.width) || 0) / 8)),
        rows: Math.max(1, Math.floor((parseInt(el.style.height) || 0) / 16))
      }
    }
    fit = vi.fn(() => {
      const next = this.proposeDimensions()
      if (!this.term || !next) return
      this.term.cols = next.cols
      this.term.rows = next.rows
    })
    constructor() {
      made.fits.push(this.fit)
    }
  }
  return { FitAddon: MockFitAddon }
})

// The GPU renderers would throw on the mock terminal; a no-op keeps the swap path alive.
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    activate(): void {}
    dispose(): void {}
  }
}))
vi.mock('@xterm/addon-canvas', () => ({
  CanvasAddon: class {
    activate(): void {}
    dispose(): void {}
  }
}))

vi.mock('@xterm/addon-web-links', () => {
  class MockWebLinksAddon {}
  return { WebLinksAddon: MockWebLinksAddon }
})

vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

Object.defineProperty(window, 'api', {
  value: {
    onTerminalData: vi.fn().mockReturnValue(() => {}),
    writeTerminal: vi.fn(),
    resizeTerminal: vi.fn(),
    openExternal: vi.fn()
  },
  writable: true
})

import {
  registerSlot,
  unregisterSlot,
  setHostRoot,
  getPersistentWrapper,
  syncTerminalOverlay,
  onRegistryChange,
  getRegisteredTerminalIds,
  destroyTerminal,
  fitTerminal,
  setAllTerminalsFontSize
} from '../src/renderer/lib/terminal-registry'

/** Read lazily, so a test can move the rect it passed. */
function makeSlot(rect: Partial<DOMRect>): HTMLDivElement {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({
      top: 0,
      left: 0,
      width: 0,
      height: 0,
      right: 0,
      bottom: 0,
      x: 0,
      y: 0,
      toJSON: () => ({}),
      ...rect
    }) as DOMRect
  return el
}

describe('terminal-registry: slot / persistent-host API', () => {
  let host: HTMLDivElement

  beforeEach(() => {
    host = document.createElement('div')
    document.body.appendChild(host)
    setHostRoot(host)
  })

  afterEach(() => {
    for (const id of getRegisteredTerminalIds()) {
      destroyTerminal(id)
    }
    setHostRoot(null)
    document.body.innerHTML = ''
    vi.clearAllMocks()
  })

  it('registerSlot creates a persistent wrapper attached to the host root', () => {
    const slot = makeSlot({ width: 300, height: 200 })
    registerSlot('term-1', slot)
    const wrapper = getPersistentWrapper('term-1')
    expect(wrapper).not.toBeNull()
    expect(wrapper!.parentElement).toBe(host)
    expect(wrapper!.dataset.terminalId).toBe('term-1')
    expect(wrapper!.style.position).toBe('fixed')
  })

  it('syncTerminalOverlay positions the wrapper to match the slot rect', () => {
    const slot = makeSlot({ top: 50, left: 100, width: 400, height: 300 })
    registerSlot('term-2', slot)
    const wrapper = getPersistentWrapper('term-2')!
    expect(wrapper.style.top).toBe('50px')
    expect(wrapper.style.left).toBe('100px')
    expect(wrapper.style.width).toBe('400px')
    expect(wrapper.style.height).toBe('300px')
    expect(wrapper.style.visibility).toBe('visible')
    expect(wrapper.style.pointerEvents).toBe('auto')
  })

  it('syncTerminalOverlay hides wrapper when slot has zero dimensions', () => {
    const slot = makeSlot({ width: 0, height: 0 })
    registerSlot('term-3', slot)
    const wrapper = getPersistentWrapper('term-3')!
    expect(wrapper.style.visibility).toBe('hidden')
    expect(wrapper.style.pointerEvents).toBe('none')
  })

  it('unregisterSlot hides the wrapper', () => {
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-4', slot)
    const wrapper = getPersistentWrapper('term-4')!
    expect(wrapper.style.visibility).toBe('visible')
    unregisterSlot('term-4', slot)
    expect(wrapper.style.visibility).toBe('hidden')
    expect(wrapper.style.pointerEvents).toBe('none')
  })

  it('unregisterSlot no-ops when the passed slot is not the active one', () => {
    const slotA = makeSlot({ width: 100, height: 100 })
    const slotB = makeSlot({ width: 100, height: 100 })
    registerSlot('term-5', slotA)
    unregisterSlot('term-5', slotB)
    const wrapper = getPersistentWrapper('term-5')!
    expect(wrapper.style.visibility).toBe('visible')
  })

  it('last registerSlot wins when multiple slots register for same id', () => {
    const slotA = makeSlot({ top: 0, width: 100, height: 100 })
    const slotB = makeSlot({ top: 500, width: 200, height: 200 })
    registerSlot('term-6', slotA)
    registerSlot('term-6', slotB)
    const wrapper = getPersistentWrapper('term-6')!
    expect(wrapper.style.top).toBe('500px')
    expect(wrapper.style.width).toBe('200px')
  })

  it('onRegistryChange fires on create and destroy, stops after unsubscribe', () => {
    const listener = vi.fn()
    const unsub = onRegistryChange(listener)
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-7', slot)
    expect(listener).toHaveBeenCalledTimes(1)
    destroyTerminal('term-7')
    expect(listener).toHaveBeenCalledTimes(2)
    unsub()
    registerSlot('term-8', slot)
    expect(listener).toHaveBeenCalledTimes(2)
    destroyTerminal('term-8')
    expect(listener).toHaveBeenCalledTimes(2)
  })

  it('setHostRoot re-parents existing wrappers to the new root', () => {
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-9', slot)
    const wrapper = getPersistentWrapper('term-9')!
    expect(wrapper.parentElement).toBe(host)
    const newHost = document.createElement('div')
    document.body.appendChild(newHost)
    setHostRoot(newHost)
    expect(wrapper.parentElement).toBe(newHost)
  })

  it('destroyTerminal removes the wrapper from the DOM', () => {
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-10', slot)
    const wrapper = getPersistentWrapper('term-10')!
    expect(wrapper.parentElement).toBe(host)
    destroyTerminal('term-10')
    expect(wrapper.parentElement).toBeNull()
    expect(getPersistentWrapper('term-10')).toBeNull()
  })

  it('syncTerminalOverlay is a no-op for unknown terminal ids', () => {
    expect(() => syncTerminalOverlay('does-not-exist')).not.toThrow()
  })

  it('getRegisteredTerminalIds reflects live terminals', () => {
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-a', slot)
    registerSlot('term-b', slot)
    const ids = getRegisteredTerminalIds()
    expect(ids).toContain('term-a')
    expect(ids).toContain('term-b')
    destroyTerminal('term-a')
    expect(getRegisteredTerminalIds()).not.toContain('term-a')
    destroyTerminal('term-b')
  })

  it('getRegisteredTerminalIds returns the same reference until the registry mutates', () => {
    const slot = makeSlot({ width: 100, height: 100 })
    registerSlot('term-cache-a', slot)
    const first = getRegisteredTerminalIds()
    const second = getRegisteredTerminalIds()
    expect(first).toBe(second)
    registerSlot('term-cache-b', slot)
    const third = getRegisteredTerminalIds()
    expect(third).not.toBe(first)
  })

  it('rounds slot rect to integer pixels so subpixel jitter is ignored', () => {
    const el = document.createElement('div')
    let rect: DOMRect = {
      top: 50.2,
      left: 100.7,
      width: 400.4,
      height: 300.6,
      right: 500,
      bottom: 350,
      x: 0,
      y: 0,
      toJSON: () => ({})
    } as DOMRect
    el.getBoundingClientRect = () => rect
    registerSlot('term-round', el)
    const wrapper = getPersistentWrapper('term-round')!
    expect(wrapper.style.top).toBe('50px')
    expect(wrapper.style.left).toBe('101px')
    expect(wrapper.style.width).toBe('400px')
    expect(wrapper.style.height).toBe('301px')
    // Subpixel jitter within the same integer — wrapper styles should stay put.
    rect = { ...rect, top: 49.9, left: 101.3 } as DOMRect
    syncTerminalOverlay('term-round')
    expect(wrapper.style.top).toBe('50px')
    expect(wrapper.style.left).toBe('101px')
  })
})

// The box moves every frame; the grid, the pty and the server's model take its size together once it settles.
describe('when a moved box becomes a size', () => {
  let host: HTMLDivElement
  let rect: Partial<DOMRect>
  const resize = (): ReturnType<typeof vi.fn> =>
    window.api.resizeTerminal as ReturnType<typeof vi.fn>
  const sizes = (): Array<{ cols: number; rows: number }> =>
    resize().mock.calls.map((c) => ({ cols: c[0].cols, rows: c[0].rows }))
  const fit = (): ReturnType<typeof vi.fn> => made.fits[made.fits.length - 1]!
  const move = (width: number, height: number): void => {
    rect.width = width
    rect.height = height
    syncTerminalOverlay('t')
  }

  beforeEach(() => {
    vi.useFakeTimers()
    made.terms.length = 0
    made.fits.length = 0
    host = document.createElement('div')
    document.body.appendChild(host)
    setHostRoot(host)
    rect = { width: 800, height: 480 }
    registerSlot('t', makeSlot(rect))
  })

  afterEach(() => {
    for (const id of getRegisteredTerminalIds()) destroyTerminal(id)
    setHostRoot(null)
    document.body.innerHTML = ''
    vi.clearAllMocks()
    vi.useRealTimers()
  })

  it('takes the first box at once', () => {
    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])
  })

  it('lets a drag settle, then refits once and tells the pty once, at the size it ended at', () => {
    const fitsBefore = fit().mock.calls.length
    for (let w = 808; w <= 880; w += 8) {
      move(w, 480)
      vi.advanceTimersByTime(16)
    }
    expect(fit().mock.calls.length).toBe(fitsBefore)
    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])

    vi.advanceTimersByTime(100)
    expect(fit().mock.calls.length).toBe(fitsBefore + 1)
    expect(sizes()).toEqual([
      { cols: 100, rows: 30 },
      { cols: 110, rows: 30 }
    ])
  })

  it('ignores a move that would not change the grid, and drops a hold it had armed', () => {
    move(880, 480)
    // Back inside the same cell count before the hold lapses: nothing to take.
    move(803, 480)

    vi.advanceTimersByTime(400)
    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])
  })

  it('fits a box that never stands still by the deadline', () => {
    // Two pixels a frame: the grid first moves at 808px, and the deadline runs from there.
    let w = 800
    for (let t = 0; t < 600; t += 16) {
      w += 2
      move(w, 480)
      vi.advanceTimersByTime(16)
    }

    expect(sizes()).toHaveLength(2)
    expect(sizes()[1]!.cols).toBeGreaterThan(100)
  })

  it("does not take a held size for the terminal's own reply to a query", () => {
    move(880, 480)
    const onData = made.terms[made.terms.length - 1]!.onData.mock.calls[0][0] as (d: string) => void
    onData('\x1b[24;80R')

    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])
  })

  it('says nothing to the pty when a drag ends where it began', () => {
    move(880, 480)
    move(800, 480)

    vi.advanceTimersByTime(100)
    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])
  })

  it('says nothing for a terminal destroyed during the hold', () => {
    move(880, 480)
    destroyTerminal('t')

    vi.advanceTimersByTime(100)
    expect(sizes()).toEqual([{ cols: 100, rows: 30 }])
  })

  it('takes a held size before a keystroke, so what is typed follows the SIGWINCH', () => {
    move(880, 480)
    const onData = made.terms[made.terms.length - 1]!.onData.mock.calls[0][0] as (d: string) => void
    onData('x')

    expect(sizes()).toEqual([
      { cols: 100, rows: 30 },
      { cols: 110, rows: 30 }
    ])
    const write = window.api.writeTerminal as ReturnType<typeof vi.fn>
    expect(resize().mock.invocationCallOrder[1]).toBeLessThan(write.mock.invocationCallOrder[0])
  })

  it('refits at once for a renderer or font that changed the cell, and holds a font-size gesture', () => {
    // The box did not move; the cell did. `fitTerminal` is that path, and it is one shot.
    getPersistentWrapper('t')!.style.width = '880px'
    fitTerminal('t')
    expect(sizes()).toEqual([
      { cols: 100, rows: 30 },
      { cols: 110, rows: 30 }
    ])

    // A pinch is many steps; the pty hears where it ends.
    getPersistentWrapper('t')!.style.width = '960px'
    setAllTerminalsFontSize(14)
    expect(sizes()).toHaveLength(2)
    vi.advanceTimersByTime(100)
    expect(sizes()[2]).toEqual({ cols: 120, rows: 30 })
  })
})
