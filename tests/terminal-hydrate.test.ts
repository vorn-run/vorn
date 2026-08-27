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

type Chunk = { id: string; data: string; seq: number }
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
  registerStatusHandler,
  initGlobalDataListener,
  disposeGlobalDataListener
} from '../src/renderer/lib/terminal-registry'

/**
 * Seeding a pane with a terminal it did not create.
 *
 * The ordering here is the whole feature and every way of getting it wrong is
 * invisible at the time: bytes applied ahead of the seed are overwritten by it,
 * bytes the seed already contains are printed twice, and bytes that arrive while
 * the request is in flight are simply gone. None of that shows until somebody
 * scrolls back.
 */

const ID = 'a-session'

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

/** What was written into this session's terminal, in order. */
function writes(): string[] {
  return created[created.length - 1]!.write.mock.calls.map((c) => c[0] as string)
}

beforeEach(() => {
  created.length = 0
  attachTerminal.mockReset()
  initGlobalDataListener()
  registerSlot(ID, slot())
})

afterEach(() => {
  destroyTerminal(ID)
  disposeGlobalDataListener()
})

describe('what the seed already contains', () => {
  it('is not applied a second time', async () => {
    // The chunk arrives while the request is in flight and is numbered at or
    // below what comes back, so the scrollback already holds it.
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = hydrateTerminal(ID)
    emit({ id: ID, data: 'already in the seed', seq: 5 })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes()).toEqual(['THE SEED'])
  })

  it('does not swallow what came after it', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = hydrateTerminal(ID)
    emit({ id: ID, data: 'in the seed', seq: 5 })
    emit({ id: ID, data: 'after the seed', seq: 6 })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes()).toEqual(['THE SEED', 'after the seed'])
  })

  it('keeps later chunks in the order they happened', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = hydrateTerminal(ID)
    for (const [data, seq] of [
      ['first', 6],
      ['second', 7],
      ['third', 8]
    ] as const) {
      emit({ id: ID, data, seq })
    }
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes()).toEqual(['THE SEED', 'first', 'second', 'third'])
  })
})

describe('nothing is written ahead of the seed', () => {
  it('holds live output from before the request was answered', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = hydrateTerminal(ID)
    emit({ id: ID, data: 'arrived first', seq: 9 })
    await frame()

    // Nothing yet: writing this now would put it above a seed that has not
    // landed, and the seed would then paint over it.
    expect(writes()).toEqual([])

    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating
    expect(writes()).toEqual(['THE SEED', 'arrived first'])
  })
})

describe('the bell', () => {
  it('does not ring for output that is only being replayed', async () => {
    // The one thing that scans terminal output fires a desktop notification on
    // \x07. A restored screen holding a bell an agent rang an hour ago must not
    // announce itself as though it had just happened.
    const handler = vi.fn()
    registerStatusHandler(ID, handler)
    attachTerminal.mockResolvedValue({ data: 'ding \x07 ding', seq: 3, live: false })

    await hydrateTerminal(ID)

    expect(handler).not.toHaveBeenCalled()
  })

  it('rings for a bell that arrived while the seed was in flight', async () => {
    // Those bytes are live -- they rang a moment ago. Holding them back for the
    // length of a round trip and then writing them silently would drop the
    // notification for the one case where the pane was not yet looking.
    const handler = vi.fn()
    registerStatusHandler(ID, handler)
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = hydrateTerminal(ID)
    emit({ id: ID, data: 'attention \x07', seq: 9 })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(handler).toHaveBeenCalledWith('attention \x07')
  })

  it('still rings for output that is actually arriving', async () => {
    const handler = vi.fn()
    registerStatusHandler(ID, handler)
    attachTerminal.mockResolvedValue({ data: 'seed', seq: 3, live: true })
    await hydrateTerminal(ID)

    emit({ id: ID, data: 'live \x07', seq: 4 })
    await frame()

    expect(handler).toHaveBeenCalledWith('live \x07')
  })
})

describe('asking twice', () => {
  it('seeds once, however many panes ask', async () => {
    // A second window, a reconnect and a re-render all land here. Seeding twice
    // would double the scrollback.
    attachTerminal.mockResolvedValue({ data: 'THE SEED', seq: 1, live: true })

    await hydrateTerminal(ID)
    await hydrateTerminal(ID)

    expect(attachTerminal).toHaveBeenCalledTimes(1)
    expect(writes()).toEqual(['THE SEED'])
  })
})

describe('a seed that never arrives', () => {
  it('lets the held output through rather than losing it', async () => {
    attachTerminal.mockRejectedValue(new Error('the server went away'))
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {})

    const hydrating = hydrateTerminal(ID)
    emit({ id: ID, data: 'happened anyway', seq: 2 })
    await frame()
    await hydrating
    quiet.mockRestore()

    expect(writes()).toEqual(['happened anyway'])
  })
})
