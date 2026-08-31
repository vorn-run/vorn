// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, cleanup, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const registryMocks = vi.hoisted(() => ({
  getTerminalBufferMetrics: vi.fn(),
  scrollTerminalToLine: vi.fn(),
  highlightTerminalBlock: vi.fn(),
  onTerminalRowHover: vi.fn(() => () => {}),
  onTerminalReady: vi.fn((_id: string, cb: () => void) => {
    cb()
    return () => {}
  }),
  onTerminalScroll: vi.fn(() => () => {})
}))
vi.mock('../src/renderer/lib/terminal-registry', () => registryMocks)

const blockMocks = vi.hoisted(() => ({
  getCommandBlocks: vi.fn(() => [] as unknown[]),
  getRunningBlock: vi.fn(() => null),
  onCommandBlocksChange: vi.fn(() => () => {}),
  formatDuration: (ms: number) => `${ms}ms`
}))
vi.mock('../src/renderer/lib/command-blocks', () => blockMocks)

import { CommandSpine } from '../src/renderer/components/CommandSpine'
import { useAppStore } from '../src/renderer/stores'
import type { TerminalSession } from '../src/shared/types'

const initialState = useAppStore.getState()

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'shell',
    projectName: 'vorn',
    projectPath: '/tmp/vorn',
    status: 'running',
    createdAt: 0,
    pid: 1,
    ...over
  }
}

function seed(over: Partial<TerminalSession> = {}) {
  act(() => {
    useAppStore.setState({
      ...initialState,
      terminals: new Map([
        [
          'term-1',
          { id: 'term-1', session: session(over), status: 'running', lastOutputTimestamp: 1 }
        ]
      ])
    })
  })
}

function marker(line: number) {
  return { line, isDisposed: false, dispose: () => {}, onDispose: () => {} }
}

beforeEach(() => {
  // jsdom has no layout, so clientHeight is 0 — stub it so the spine has a
  // height to map marks onto.
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get: () => 200
  })
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
  registryMocks.getTerminalBufferMetrics.mockReturnValue({
    length: 201,
    viewportY: 0,
    baseY: 177,
    rows: 24,
    cursorLine: 200,
    isAlternate: false
  })
  blockMocks.getCommandBlocks.mockReturnValue([])
  blockMocks.getRunningBlock.mockReturnValue(null)
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('CommandSpine', () => {
  it('renders nothing for an agent session', () => {
    // The contract that keeps agent cards untouched: their TUI owns the
    // screen and has no command boundaries to mark.
    seed({ agentType: 'claude' })
    const { container } = render(<CommandSpine terminalId="term-1" className="" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('renders one mark per command, labelled with the command and exit code', () => {
    seed()
    blockMocks.getCommandBlocks.mockReturnValue([
      { command: 'git status', exitCode: 0, durationMs: 200, outputLines: 2, marker: marker(0) },
      { command: 'yarn test', exitCode: 1, durationMs: 4100, outputLines: 40, marker: marker(10) }
    ])
    render(<CommandSpine terminalId="term-1" className="" />)
    expect(screen.getByLabelText('git status · exit 0')).toBeInTheDocument()
    expect(screen.getByLabelText('yarn test · exit 1')).toBeInTheDocument()
  })

  it('scrolls the terminal to the command when a mark is clicked', () => {
    seed()
    blockMocks.getCommandBlocks.mockReturnValue([
      { command: 'yarn build', exitCode: 0, durationMs: 900, outputLines: 3, marker: marker(12) }
    ])
    render(<CommandSpine terminalId="term-1" className="" />)
    fireEvent.click(screen.getByLabelText('yarn build · exit 0'))
    expect(registryMocks.scrollTerminalToLine).toHaveBeenCalledWith('term-1', 12)
  })

  it('draws no marks while a full-screen application owns the buffer', () => {
    seed()
    registryMocks.getTerminalBufferMetrics.mockReturnValue({
      length: 201,
      viewportY: 0,
      baseY: 177,
      rows: 24,
      cursorLine: 200,
      isAlternate: true
    })
    blockMocks.getCommandBlocks.mockReturnValue([
      { command: 'git status', exitCode: 0, durationMs: 200, outputLines: 2, marker: marker(0) }
    ])
    render(<CommandSpine terminalId="term-1" className="" />)
    expect(screen.queryByLabelText(/git status/)).not.toBeInTheDocument()
  })

  it('shows the command on hover, in a tooltip portalled out of the card', () => {
    seed()
    blockMocks.getCommandBlocks.mockReturnValue([
      { command: 'yarn lint', exitCode: 0, durationMs: 1800, outputLines: 2, marker: marker(5) }
    ])
    const { container } = render(<CommandSpine terminalId="term-1" className="" />)
    fireEvent.mouseEnter(screen.getByLabelText('yarn lint · exit 0'))
    const tip = screen.getByText('yarn lint')
    expect(tip).toBeInTheDocument()
    // Portalled: it must not live inside the spine, which sits beside the
    // terminal's fixed overlay layer.
    expect(container.contains(tip)).toBe(false)
  })
})

describe('CommandSpine block highlight', () => {
  it('lights up the block rows on hover and clears them on leave', () => {
    seed()
    blockMocks.getCommandBlocks.mockReturnValue([
      { command: 'yarn test', exitCode: 1, durationMs: 4100, outputLines: 40, marker: marker(0) },
      { command: 'git status', exitCode: 0, durationMs: 200, outputLines: 2, marker: marker(10) }
    ])
    const { container } = render(<CommandSpine terminalId="term-1" className="" />)

    fireEvent.mouseEnter(screen.getByLabelText('yarn test · exit 1'))
    expect(registryMocks.highlightTerminalBlock).toHaveBeenCalledWith('term-1', {
      startLine: 0,
      endLine: 9
    })

    fireEvent.mouseLeave(container.firstChild as HTMLElement)
    expect(registryMocks.highlightTerminalBlock).toHaveBeenLastCalledWith('term-1', null)
  })
})
