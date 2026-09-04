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

/**
 * Open the pane, which is what starts the seed.
 *
 * Production hydrates when the terminal is built rather than when somebody asks,
 * so the mock has to be armed before this and not after. Awaiting the returned
 * promise joins the seed already in flight; it does not start a second.
 */
function open(): Promise<void> {
  registerSlot(ID, slot())
  return hydrateTerminal(ID)
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

describe('what the seed already contains', () => {
  it('is not applied a second time', async () => {
    // The chunk arrives while the request is in flight and is numbered at or
    // below what comes back, so the scrollback already holds it.
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = open()
    emit({ id: ID, data: 'already in the seed', seq: 5 })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes()).toEqual(['THE SEED'])
  })

  it('does not swallow what came after it', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = open()
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

    const hydrating = open()
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

    // One write, not three. xterm queues a task per call, and the held chunks
    // are already in order -- joining them is the same bytes at a third of the
    // scheduling.
    expect(writes()).toEqual(['THE SEED', 'firstsecondthird'])
  })
})

describe('nothing is written ahead of the seed', () => {
  it('holds live output from before the request was answered', async () => {
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = open()
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

describe('asking twice', () => {
  it('seeds once, however many panes ask', async () => {
    // A second window, a reconnect and a re-render all land here. Seeding twice
    // would double the scrollback.
    attachTerminal.mockResolvedValue({ data: 'THE SEED', seq: 1, live: true })

    await open()
    await hydrateTerminal(ID)

    expect(attachTerminal).toHaveBeenCalledTimes(1)
    expect(writes()).toEqual(['THE SEED'])
  })
})

describe('a seed that never arrives', () => {
  it('lets the held output through rather than losing it', async () => {
    attachTerminal.mockRejectedValue(new Error('the server went away'))
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {})

    const hydrating = open()
    emit({ id: ID, data: 'happened anyway', seq: 2 })
    await frame()
    await hydrating
    quiet.mockRestore()

    expect(writes()).toEqual(['happened anyway'])
  })
})

describe('a chunk whose sequence cannot be compared', () => {
  it('is shown rather than dropped without a word', async () => {
    // `seq` is required by the protocol, so this means a server not keeping to
    // it. Filtering on `undefined > 5` is false for every chunk, so the whole
    // attach window went missing -- silently, which is the one way this must
    // never fail. Showing it twice would at least be visible.
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = open()
    emit({ id: ID, data: 'no sequence on this', seq: undefined as unknown as number })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes().join('')).toContain('no sequence on this')
  })

  it('still drops what the seed already contained, when the sequence is usable', async () => {
    // The contrast, so the guard above cannot quietly become "keep everything".
    let answer: (v: unknown) => void = () => {}
    attachTerminal.mockReturnValue(new Promise((r) => (answer = r)))

    const hydrating = open()
    emit({ id: ID, data: 'already in the seed', seq: 5 })
    emit({ id: ID, data: 'after the seed', seq: 6 })
    await frame()
    answer({ data: 'THE SEED', seq: 5, live: true })
    await hydrating

    expect(writes().join('')).not.toContain('already in the seed')
    expect(writes().join('')).toContain('after the seed')
  })
})
