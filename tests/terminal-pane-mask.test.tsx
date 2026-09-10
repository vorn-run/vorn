// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup } from '@testing-library/react'

// The live region is a window onto a grid fitted to the pane; a full-screen program gets the pane itself.
const hoisted = vi.hoisted(() => ({
  rows: 2 as number | null,
  slots: [] as Array<{ fitTo?: { current: HTMLElement | null } }>
}))

vi.mock('../src/renderer/hooks/useLiveTerminalRows', () => ({
  useLiveTerminalRows: () => hoisted.rows
}))
vi.mock('../src/renderer/lib/command-blocks', () => ({
  hasShellIntegration: () => true,
  onCommandBlocksChange: () => () => {}
}))
vi.mock('../src/renderer/lib/block-log', () => ({ registerBlockLogView: () => () => {} }))
vi.mock('../src/renderer/components/BlockLog', () => ({ BlockLog: () => null }))
vi.mock('../src/renderer/components/RunningCommand', () => ({ RunningCommand: () => null }))
vi.mock('../src/renderer/components/CommandSpine', () => ({ CommandSpine: () => null }))
vi.mock('../src/renderer/components/TerminalSlot', () => ({
  TerminalSlot: (props: { fitTo?: { current: HTMLElement | null } }) => {
    hoisted.slots.push(props)
    return <div data-testid="slot" />
  }
}))

import { TerminalPane } from '../src/renderer/components/TerminalPane'

afterEach(() => {
  cleanup()
  hoisted.slots.length = 0
})

describe('the live region in block mode', () => {
  it('is a window onto a grid fitted to the pane', () => {
    hoisted.rows = 2
    const { container } = render(
      <TerminalPane terminalId="t" agentType="shell" isFocused={false} domBlocks />
    )

    const slot = hoisted.slots.at(-1)!
    expect(slot.fitTo?.current).toBe(container.firstElementChild)
  })

  it('is the pane itself for a full-screen program', () => {
    hoisted.rows = null
    render(<TerminalPane terminalId="t" agentType="shell" isFocused={false} domBlocks />)

    expect(hoisted.slots.at(-1)!.fitTo).toBeUndefined()
  })
})
