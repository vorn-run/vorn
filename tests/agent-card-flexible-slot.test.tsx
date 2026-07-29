// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Stubs that AgentCard reads at module load; installed before the import below.
vi.hoisted(() => {
  Object.defineProperty(window, 'api', {
    value: {
      isWorktreeDirty: () => Promise.resolve(false),
      getGitDiffStat: () => Promise.resolve(null),
      getGitBranch: () => Promise.resolve(null),
      notifyWidgetStatus: () => {},
      detectIDEs: () => Promise.resolve([]),
      openInIDE: () => {}
    },
    writable: true
  })
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

const slotCalls: Array<{ terminalId: string; className?: string }> = []

vi.mock('../src/renderer/components/TerminalSlot', () => ({
  TerminalSlot: (props: { terminalId: string; className?: string }) => {
    slotCalls.push({ terminalId: props.terminalId, className: props.className })
    return <div data-testid={`slot-${props.terminalId}`} className={props.className} />
  }
}))

vi.mock('../src/renderer/hooks/useTerminalScrollButton', () => ({
  useTerminalScrollButton: () => ({ showScrollBtn: false, handleScrollToBottom: () => {} })
}))

// Whether the shell reports command boundaries. Blocks are gated on it, so a
// layout test has to state which kind of shell it is describing.
let integrated = false
vi.mock('../src/renderer/lib/command-blocks', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, hasShellIntegration: () => integrated }
})

import { useAppStore } from '../src/renderer/stores'
import { AgentCard } from '../src/renderer/components/AgentCard'

function seedTerminal(
  id: string,
  agentType: 'claude' | 'shell' = 'claude',
  domBlockRendering = false
) {
  const terminals = new Map()
  terminals.set(id, {
    id,
    session: {
      id,
      agentType,
      projectName: 'Vorn',
      projectPath: '/tmp/vorn',
      isWorktree: false,
      branch: 'main',
      createdAt: Date.now()
    },
    status: 'idle',
    lastOutputTimestamp: Date.now()
  })
  useAppStore.setState({
    config: { defaults: { domBlockRendering } } as never,
    terminals,
    focusedTerminalId: null,
    selectedTerminalId: null,
    renamingTerminalId: null,
    minimizedTerminals: new Set<string>()
  })
}

beforeEach(() => {
  slotCalls.length = 0
  integrated = false
  seedTerminal('t1')
})

afterEach(() => cleanup())

describe('AgentCard TerminalSlot sizing', () => {
  it('uses w-full h-full in tab/grid mode', () => {
    render(<AgentCard terminalId="t1" />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('w-full h-full')
  })

  it('reserves 16px SE in flexible mode so the resize handle stays reachable', () => {
    render(<AgentCard terminalId="t1" flexible />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('absolute inset-0 right-4 bottom-4')
  })

  it('leaves room for the spine on a shell session', () => {
    seedTerminal('t1', 'shell')
    render(<AgentCard terminalId="t1" />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('flex-1 min-w-0 h-full')
  })

  it('keeps the SE reservation alongside the spine in flexible mode', () => {
    // left-4 clears the 8px spine plus its 8px gap; right-6 keeps the block
    // rules off the edge while preserving the 16px resize-handle corner.
    seedTerminal('t1', 'shell')
    render(<AgentCard terminalId="t1" flexible />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('absolute inset-0 left-4 right-6 bottom-4')
  })

  it('gives the terminal a fixed live region when blocks are drawn as elements', () => {
    integrated = true
    seedTerminal('t1', 'shell', true)
    render(<AgentCard terminalId="t1" />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('shrink-0 w-full')
  })

  it('keeps the SE reservation on the wrapper in flexible block mode', () => {
    integrated = true
    // The resize handle must stay reachable even though the terminal itself
    // is now a flex child rather than the absolutely positioned element.
    seedTerminal('t1', 'shell', true)
    const { container } = render(<AgentCard terminalId="t1" flexible />)
    expect(container.querySelector('.absolute.inset-0.right-4.bottom-4')).not.toBeNull()
  })

  it('leaves the terminal alone in a shell that reports no command boundaries', () => {
    // bash, fish, PowerShell and cmd never send OSC 133, so there are no
    // blocks to draw. Splitting the pane anyway would cap the terminal at the
    // live region's height with nothing above it.
    integrated = false
    seedTerminal('t1', 'shell', true)
    render(<AgentCard terminalId="t1" />)
    const call = slotCalls.find((c) => c.terminalId === 't1')
    expect(call?.className).toBe('flex-1 min-w-0 h-full')
  })
})
