// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const hoisted = vi.hoisted(() => {
  const stopSync = vi.fn()
  return {
    setHostRoot: vi.fn(),
    stopSync,
    startTerminalOverlaySync: vi.fn(() => stopSync),
    TERMINAL_ID_ATTR: 'data-terminal-id'
  }
})

vi.mock('../src/renderer/lib/terminal-registry', () => hoisted)

vi.mock('../src/renderer/components/TerminalContextMenu', () => ({
  TerminalContextMenu: ({ terminalId, onClose }: { terminalId: string; onClose: () => void }) => (
    <div data-testid="context-menu" data-terminal-id={terminalId} onClick={onClose} />
  )
}))

import { TerminalHost } from '../src/renderer/components/TerminalHost'

describe('TerminalHost', () => {
  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  it('sets the host root on mount and clears it on unmount', () => {
    const { unmount } = render(<TerminalHost />)
    expect(hoisted.setHostRoot).toHaveBeenCalledWith(expect.any(HTMLElement))
    unmount()
    expect(hoisted.setHostRoot).toHaveBeenLastCalledWith(null)
  })

  it('renders a fixed-position root div with pointer-events disabled', () => {
    const { container } = render(<TerminalHost />)
    const root = container.querySelector('div.fixed.inset-0') as HTMLElement
    expect(root).not.toBeNull()
    expect(root.className).toContain('fixed')
    expect(root.className).toContain('pointer-events-none')
  })

  it('keeps the wrappers on their slots from mount to unmount', () => {
    const { unmount } = render(<TerminalHost />)
    const root = hoisted.setHostRoot.mock.calls[0][0]
    expect(hoisted.startTerminalOverlaySync).toHaveBeenCalledWith(root)
    expect(hoisted.stopSync).not.toHaveBeenCalled()
    unmount()
    expect(hoisted.stopSync).toHaveBeenCalledTimes(1)
  })

  it('opens the context menu on right-click and closes it via onClose', () => {
    const { container, getByTestId, queryByTestId } = render(<TerminalHost />)
    const root = container.querySelector('div.fixed.inset-0') as HTMLElement
    const wrapper = document.createElement('div')
    wrapper.dataset.terminalId = 'my-term'
    root.appendChild(wrapper)
    fireEvent.contextMenu(wrapper, { clientX: 100, clientY: 200 })
    const menu = getByTestId('context-menu')
    expect(menu).toHaveAttribute('data-terminal-id', 'my-term')
    fireEvent.click(menu)
    expect(queryByTestId('context-menu')).toBeNull()
  })

  it('ignores right-clicks on elements without data-terminal-id', () => {
    const { container, queryByTestId } = render(<TerminalHost />)
    const root = container.querySelector('div.fixed.inset-0') as HTMLElement
    const stray = document.createElement('div')
    root.appendChild(stray)
    fireEvent.contextMenu(stray, { clientX: 0, clientY: 0 })
    expect(queryByTestId('context-menu')).toBeNull()
  })
})
