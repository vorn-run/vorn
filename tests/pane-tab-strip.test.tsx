// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import { PaneTabStrip, type PaneTab } from '../src/renderer/components/PaneTabStrip'

const TABS: PaneTab[] = [
  { id: 'a', name: 'a.ts' },
  { id: 'b', name: 'b.ts', italic: true },
  { id: 'c', name: 'c.ts', badge: <span data-testid="dot" /> }
]

function strip(over: Partial<Parameters<typeof PaneTabStrip>[0]> = {}) {
  const onSelect = vi.fn()
  const onClose = vi.fn()
  render(
    <PaneTabStrip
      tabs={TABS}
      activeId="a"
      onSelect={onSelect}
      onClose={onClose}
      ariaLabel="Open files"
      testId="strip"
      trailing={<button aria-label="Close pane" />}
      {...over}
    />
  )
  return { onSelect, onClose }
}

afterEach(() => vi.restoreAllMocks())

describe('PaneTabStrip', () => {
  it('marks the tab in front and selects another on click or from the keyboard', () => {
    const { onSelect } = strip()
    expect(screen.getByRole('tab', { name: /a\.ts/ })).toHaveAttribute('aria-selected', 'true')

    fireEvent.click(screen.getByRole('tab', { name: /b\.ts/ }))
    fireEvent.keyDown(screen.getByRole('tab', { name: /c\.ts/ }), { key: 'Enter' })
    expect(onSelect.mock.calls).toEqual([['b'], ['c']])
  })

  it('closes a tab without selecting it', () => {
    const { onSelect, onClose } = strip()
    fireEvent.click(screen.getByRole('button', { name: 'Close b.ts' }))
    expect(onClose).toHaveBeenCalledWith('b')
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('hands the whole strip to the drag handler, and keeps a tab press from the card when asked', () => {
    const onPointerDown = vi.fn()
    strip({ onPointerDown, draggable: true, isolateTabPointer: true })

    fireEvent.pointerDown(screen.getByTestId('strip'))
    expect(onPointerDown).toHaveBeenCalledOnce()
    expect(screen.getByTestId('strip').className).toContain('drag-handle')

    fireEvent.pointerDown(screen.getByRole('tab', { name: /a\.ts/ }))
    expect(onPointerDown).toHaveBeenCalledOnce()
  })

  it('keeps a double-click on a tab from reaching the strip', () => {
    const onDoubleClick = vi.fn()
    const onDoubleClickTab = vi.fn()
    strip({ onDoubleClick, onDoubleClickTab })

    fireEvent.doubleClick(screen.getByRole('tab', { name: /b\.ts/ }))
    expect(onDoubleClickTab).toHaveBeenCalledWith('b')
    expect(onDoubleClick).not.toHaveBeenCalled()
  })

  it('draws a preview tab in italics and a badge where one is given', () => {
    strip()
    expect(screen.getByRole('tab', { name: /b\.ts/ }).querySelector('.italic')).not.toBeNull()
    expect(screen.getByTestId('dot')).toBeInTheDocument()
  })

  it('names the pane where its tabs would be while it has none', () => {
    strip({ tabs: [], activeId: null, emptyTitle: 'Files' })
    expect(screen.getByRole('tablist')).toHaveTextContent('Files')
  })

  it('offers the list of all tabs only when they do not fit', () => {
    strip()
    expect(screen.queryByRole('button', { name: 'All open tabs' })).toBeNull()
  })

  it('lists every tab when they overflow, and selects from the list', () => {
    // jsdom measures nothing, so the scroller is told its tabs are wider than it.
    const scrollWidth = vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(900)
    const footer = vi.fn()
    const { onSelect } = strip({
      menuFooter: (close) => (
        <button
          role="menuitem"
          onClick={() => {
            footer()
            close()
          }}
        >
          Close saved tabs
        </button>
      )
    })
    scrollWidth.mockRestore()

    fireEvent.click(screen.getByRole('button', { name: 'All open tabs' }))
    const menu = screen.getByRole('menu')
    expect(menu).toHaveTextContent('a.ts')
    expect(menu).toHaveTextContent('c.ts')

    fireEvent.click(screen.getByRole('menuitem', { name: /c\.ts/ }))
    expect(onSelect).toHaveBeenCalledWith('c')
    expect(screen.queryByRole('menu')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'All open tabs' }))
    act(() => fireEvent.click(screen.getByRole('menuitem', { name: 'Close saved tabs' })))
    expect(footer).toHaveBeenCalledOnce()
    expect(screen.queryByRole('menu')).toBeNull()
  })
})
