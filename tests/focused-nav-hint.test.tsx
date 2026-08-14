// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>
}))

import { useAppStore } from '../src/renderer/stores'
import { FocusedNavHint } from '../src/renderer/components/card/FocusedNavHint'

beforeEach(() => {
  useAppStore.setState({
    focusableTerminalIds: ['a', 'b', 'c']
  })
})

describe('FocusedNavHint', () => {
  it('renders "N / total" with the 1-based index of the current terminal', () => {
    const { container } = render(<FocusedNavHint terminalId="b" />)
    expect(container.textContent).toContain('2')
    expect(container.textContent).toContain('3')
    expect(container.textContent).toMatch(/2\s*\/\s*3/)
  })

  it('returns nothing when fewer than 2 sessions are focusable', () => {
    useAppStore.setState({ focusableTerminalIds: ['a'] })
    const { container } = render(<FocusedNavHint terminalId="a" />)
    expect(container.firstChild).toBeNull()
  })

  it('returns nothing when the current terminal is not in the focusable list', () => {
    const { container } = render(<FocusedNavHint terminalId="gone" />)
    expect(container.firstChild).toBeNull()
  })

  it('Next button focuses the following terminal', () => {
    const setFocusedTerminal = vi.fn()
    useAppStore.setState({ setFocusedTerminal })
    render(<FocusedNavHint terminalId="a" />)
    fireEvent.click(screen.getByLabelText('Next cell'))
    expect(setFocusedTerminal).toHaveBeenCalledWith('b')
  })

  it('Next button wraps around from the last terminal to the first', () => {
    const setFocusedTerminal = vi.fn()
    useAppStore.setState({ setFocusedTerminal })
    render(<FocusedNavHint terminalId="c" />)
    fireEvent.click(screen.getByLabelText('Next cell'))
    expect(setFocusedTerminal).toHaveBeenCalledWith('a')
  })

  it('Previous button focuses the preceding terminal', () => {
    const setFocusedTerminal = vi.fn()
    useAppStore.setState({ setFocusedTerminal })
    render(<FocusedNavHint terminalId="c" />)
    fireEvent.click(screen.getByLabelText('Previous cell'))
    expect(setFocusedTerminal).toHaveBeenCalledWith('b')
  })

  it('Previous button wraps from the first terminal to the last', () => {
    const setFocusedTerminal = vi.fn()
    useAppStore.setState({ setFocusedTerminal })
    render(<FocusedNavHint terminalId="a" />)
    fireEvent.click(screen.getByLabelText('Previous cell'))
    expect(setFocusedTerminal).toHaveBeenCalledWith('c')
  })

  it('button pointerDown does not bubble (prevents drag interference)', () => {
    const parentPointerDown = vi.fn()
    render(
      <div onPointerDown={parentPointerDown}>
        <FocusedNavHint terminalId="b" />
      </div>
    )
    fireEvent.pointerDown(screen.getByLabelText('Next cell'))
    fireEvent.pointerDown(screen.getByLabelText('Previous cell'))
    expect(parentPointerDown).not.toHaveBeenCalled()
  })
})
