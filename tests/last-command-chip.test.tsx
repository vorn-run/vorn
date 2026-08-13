// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const blocks: {
  command: string | null
  exitCode: number
  durationMs: number
  marker: { line: number; isDisposed: boolean }
}[] = []

vi.mock('../src/renderer/lib/command-blocks', async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    '../src/renderer/lib/command-blocks'
  )
  return {
    ...actual,
    getCommandBlocks: () => blocks,
    onCommandBlocksChange: () => () => {}
  }
})
vi.mock('../src/renderer/lib/terminal-registry', () => ({
  onTerminalReady: (_id: string, cb: () => void) => {
    cb()
    return () => {}
  },
  scrollTerminalToLine: () => {}
}))

import { useAppStore } from '../src/renderer/stores'
import { LastCommandChip } from '../src/renderer/components/card/LastCommandChip'

function seed(): void {
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        [
          't1',
          {
            id: 't1',
            session: { id: 't1', agentType: 'shell', displayName: 't1' },
            status: 'idle',
            lastOutputTimestamp: 1
          }
        ]
      ]) as never
    })
  })
}

function setBlock(exitCode: number): void {
  blocks.length = 0
  blocks.push({
    command: 'yarn test',
    exitCode,
    durationMs: 2400,
    marker: { line: 12, isDisposed: false }
  })
}

beforeEach(seed)
afterEach(() => cleanup())

describe('LastCommandChip', () => {
  it('lets a succeeded command recede', () => {
    // Success is the ordinary outcome. Colouring it puts a mark on the status
    // bar of every card that has ever run anything, which spends attention on
    // the one thing that never needs it.
    setBlock(0)
    const { container } = render(<LastCommandChip terminalId="t1" />)

    expect(screen.getByText('✓')).toHaveClass('text-ink-faint')
    expect(container.querySelector('.text-danger')).toBeNull()
  })

  it('names the exit code when a command failed', () => {
    // The failure is the whole reason to look at this chip, so it is the one
    // state that keeps a colour — and the code is what says which failure.
    setBlock(1)
    render(<LastCommandChip terminalId="t1" />)

    expect(screen.getByText('✗')).toHaveClass('text-danger')
    expect(screen.getByText('1')).toHaveClass('text-danger')
  })

  it('stays out of a non-shell session', () => {
    // Agent sessions have no command blocks to report; the chip would render an
    // empty slot in every card's status bar.
    setBlock(0)
    act(() => {
      const t = useAppStore.getState().terminals.get('t1')!
      useAppStore.setState({
        terminals: new Map([
          ['t1', { ...t, session: { ...t.session, agentType: 'claude' } }]
        ]) as never
      })
    })
    const { container } = render(<LastCommandChip terminalId="t1" />)
    expect(container).toBeEmptyDOMElement()
  })
})
