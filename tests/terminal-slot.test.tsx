// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const hoisted = vi.hoisted(() => ({
  registerSlot: vi.fn(),
  unregisterSlot: vi.fn(),
  focusTerminal: vi.fn()
}))

vi.mock('../src/renderer/lib/terminal-registry', () => hoisted)

const { registerSlot, unregisterSlot, focusTerminal } = hoisted

import { TerminalSlot } from '../src/renderer/components/TerminalSlot'

describe('TerminalSlot', () => {
  let rafCallbacks: Array<() => void> = []

  beforeEach(() => {
    rafCallbacks = []
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn((cb: () => void) => {
        rafCallbacks.push(cb)
        return rafCallbacks.length
      })
    )
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
    vi.unstubAllGlobals()
  })

  it('registers with the registry on mount and unregisters on unmount', () => {
    const { unmount } = render(
      <TerminalSlot terminalId="abc" isFocused={false} className="w-full h-full" />
    )
    expect(registerSlot).toHaveBeenCalledTimes(1)
    expect(registerSlot).toHaveBeenCalledWith('abc', expect.any(HTMLElement))
    unmount()
    expect(unregisterSlot).toHaveBeenCalledTimes(1)
    expect(unregisterSlot).toHaveBeenCalledWith('abc', expect.any(HTMLElement))
  })

  it('renders a div with the given className', () => {
    const { container } = render(
      <TerminalSlot terminalId="abc" isFocused={false} className="my-slot" />
    )
    const div = container.querySelector('div')
    expect(div).not.toBeNull()
    expect(div).toHaveClass('my-slot')
  })

  it('focuses the terminal on the next animation frame when isFocused is true', () => {
    render(<TerminalSlot terminalId="xyz" isFocused={true} />)
    expect(focusTerminal).not.toHaveBeenCalled()
    rafCallbacks.forEach((cb) => cb())
    expect(focusTerminal).toHaveBeenCalledWith('xyz')
  })

  it('does not call focusTerminal when isFocused is false', () => {
    render(<TerminalSlot terminalId="xyz" isFocused={false} />)
    rafCallbacks.forEach((cb) => cb())
    expect(focusTerminal).not.toHaveBeenCalled()
  })
})
