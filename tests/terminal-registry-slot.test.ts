// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

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
    loadAddon = vi.fn()
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
    fit = vi.fn()
  }
  return { FitAddon: MockFitAddon }
})

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
  destroyTerminal
} from '../src/renderer/lib/terminal-registry'

function makeSlot(rect: Partial<DOMRect>): HTMLDivElement {
  const el = document.createElement('div')
  const full: DOMRect = {
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
  } as DOMRect
  el.getBoundingClientRect = () => full
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
