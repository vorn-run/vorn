// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { useAppStore } from '../src/renderer/stores'
import { SessionItem } from '../src/renderer/components/project-sidebar/SessionItem'
import type { SidebarSessionInfo } from '../src/renderer/components/project-sidebar/types'

const session: SidebarSessionInfo = {
  id: 'sess-1',
  name: 'My Session',
  status: 'running',
  agentType: 'claude',
  branch: 'main',
  isWorktree: false
}

const initialState = useAppStore.getState()

describe('SessionItem', () => {
  beforeEach(() => {
    useAppStore.setState({ focusedTerminalId: null })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('renders session name', () => {
    render(<SessionItem session={session} />)
    expect(screen.getByText('My Session')).toBeInTheDocument()
  })

  it('renders branch when showBranch is true (default)', () => {
    render(<SessionItem session={session} />)
    expect(screen.getByText('main')).toBeInTheDocument()
  })

  it('hides branch when showBranch is false', () => {
    render(<SessionItem session={session} showBranch={false} />)
    expect(screen.queryByText('main')).not.toBeInTheDocument()
  })

  it('replaces the agent identity icon with the running glyph when running', () => {
    const { container } = render(<SessionItem session={session} />)
    const glyph = container.querySelector('[data-component="running-glyph"]')
    expect(glyph).toBeInTheDocument()
    expect(glyph).toHaveAttribute('aria-label', 'Running')
  })

  it('calls setFocusedTerminal on click', () => {
    const setFocused = vi.fn()
    useAppStore.setState({ setFocusedTerminal: setFocused })
    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByText('My Session'))
    expect(setFocused).toHaveBeenCalledWith('sess-1')
  })

  it('applies focused style when session is focused', () => {
    useAppStore.setState({ focusedTerminalId: 'sess-1' })
    const { container } = render(<SessionItem session={session} />)
    const button = container.querySelector('button')
    expect(button?.className).toContain('text-white')
  })

  it('applies unfocused style when session is not focused', () => {
    useAppStore.setState({ focusedTerminalId: 'other' })
    const { container } = render(<SessionItem session={session} />)
    const button = container.querySelector('button')
    expect(button?.className).toContain('text-gray-400')
  })

  it('renders without branch when session has no branch', () => {
    const noBranch: SidebarSessionInfo = { ...session, branch: undefined }
    render(<SessionItem session={noBranch} />)
    expect(screen.queryByText('main')).not.toBeInTheDocument()
  })

  it.each(['waiting', 'idle', 'error'] as const)(
    'renders the plain agent identity icon (no pulse) for %s status',
    (status) => {
      const s: SidebarSessionInfo = { ...session, status }
      const { container } = render(<SessionItem session={s} />)
      expect(container.querySelector('[data-component="running-glyph"]')).toBeNull()
      // Identity svg should be rendered inside the icon wrapper, not matched globally
      const sessionButton = screen.getByText('My Session').closest('button')
      const iconWrapper = sessionButton?.querySelector('span')
      expect(iconWrapper?.querySelector('svg')).toBeInTheDocument()
    }
  )

  it('renders close button', () => {
    const { container } = render(<SessionItem session={session} />)
    const closeBtn = container.querySelector('button[type="button"]')
    expect(closeBtn).toBeInTheDocument()
  })

  it("toggles this session's files pane without selecting the session", () => {
    const setActiveTabId = vi.fn()
    act(() => useAppStore.setState({ setActiveTabId }))

    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByRole('button', { name: /Show files for/ }))

    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(true)
    // The row is itself a button; the toggle must stop propagation or opening
    // files would also switch the active session.
    expect(setActiveTabId).not.toHaveBeenCalled()
  })

  it('reflects and clears an open files pane', () => {
    act(() => useAppStore.getState().openFilesPane(session.id))
    render(<SessionItem session={session} />)

    const btn = screen.getByRole('button', { name: /Hide files for/ })
    fireEvent.click(btn)
    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(false)
  })
})
