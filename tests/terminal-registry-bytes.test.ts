// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const { created } = vi.hoisted(() => ({
  created: [] as Array<{ id: number; write: ReturnType<typeof vi.fn> }>
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
    constructor() {
      created.push({ id: n++, write: this.write })
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
  FitAddon: class {
    fit = vi.fn()
  }
}))
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

type Chunk = { id: string; data: string | Uint8Array; seq: number }
let emit: (c: Chunk) => void = () => {}
const attachTerminal = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    onTerminalData: (cb: (c: Chunk) => void) => {
      emit = cb
      return () => {}
    },
    attachTerminal,
    writeTerminal: vi.fn(),
    resizeTerminal: vi.fn(),
    openExternal: vi.fn()
  },
  writable: true
})

import {
  registerSlot,
  destroyTerminal,
  hydrateTerminal,
  initGlobalDataListener,
  disposeGlobalDataListener
} from '../src/renderer/lib/terminal-registry'

/**
 * Terminal output arriving as bytes.
 *
 * xterm takes bytes or text but not both in one call, and the seed a pane is
 * given on attach is text while everything live is bytes. So what is pinned is
 * the joining: byte chunks from one frame go in as one write, and text never
 * ends up glued to bytes.
 */

const ID = 'a-session'
const bytes = (text: string): Uint8Array => new TextEncoder().encode(text)

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

const frame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()))

function writes(): Array<string | Uint8Array> {
  return created[created.length - 1]!.write.mock.calls.map((c) => c[0] as string | Uint8Array)
}

/** What went in, readable; bytes are marked so text and bytes cannot be confused. */
function written(): string[] {
  return writes().map((w) =>
    typeof w === 'string' ? `text:${w}` : `bytes:${new TextDecoder().decode(w)}`
  )
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

describe('terminal output as bytes, in the pane', () => {
  it('joins the byte chunks of one frame into one write', async () => {
    attachTerminal.mockResolvedValue({ data: '', seq: 0, live: true })
    registerSlot(ID, slot())
    await hydrateTerminal(ID)

    emit({ id: ID, data: bytes('ab'), seq: 1 })
    emit({ id: ID, data: bytes('cd'), seq: 2 })
    await frame()

    expect(written()).toEqual(['bytes:abcd'])
  })

  it('writes a text seed and the bytes held behind it separately, in order', async () => {
    let seed!: (v: { data: string; seq: number; live: boolean }) => void
    attachTerminal.mockReturnValue(new Promise((resolve) => (seed = resolve)))
    registerSlot(ID, slot())
    const hydrated = hydrateTerminal(ID)

    // Arrive while the seed is in flight, so they are held and applied after it.
    emit({ id: ID, data: bytes('live-1'), seq: 2 })
    emit({ id: ID, data: bytes('live-2'), seq: 3 })
    await frame()
    seed({ data: 'seed', seq: 1, live: true })
    await hydrated

    expect(written()).toEqual(['text:seed', 'bytes:live-1live-2'])
  })

  it('still takes text from a server that sends it', async () => {
    attachTerminal.mockResolvedValue({ data: '', seq: 0, live: true })
    registerSlot(ID, slot())
    await hydrateTerminal(ID)

    emit({ id: ID, data: 'plain', seq: 1 })
    emit({ id: ID, data: ' text', seq: 2 })
    await frame()

    expect(written()).toEqual(['text:plain text'])
  })
})
