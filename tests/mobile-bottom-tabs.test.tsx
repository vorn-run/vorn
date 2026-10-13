// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockStore = {
  setMainViewMode: vi.fn(),
  setSettingsOpen: vi.fn(),
  isSettingsOpen: false,
  config: { defaults: { mainViewMode: 'sessions' as const } }
}

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (s: unknown) => unknown) => (selector ? selector(mockStore) : mockStore)
}))

const { MobileBottomTabs } =
  await import('../packages/web/src/renderer/components/MobileBottomTabs')

beforeEach(() => {
  mockStore.setMainViewMode.mockReset()
  mockStore.setSettingsOpen.mockReset()
  mockStore.isSettingsOpen = false
  mockStore.config.defaults.mainViewMode = 'sessions'
})

describe('MobileBottomTabs', () => {
  it('renders Sessions, Tasks, Workflows, and Settings tabs', () => {
    render(<MobileBottomTabs />)
    expect(screen.getByText('Sessions')).toBeInTheDocument()
    expect(screen.getByText('Tasks')).toBeInTheDocument()
    expect(screen.getByText('Workflows')).toBeInTheDocument()
    expect(screen.getByText('Settings')).toBeInTheDocument()
  })

  it('switches to workflows view when the Workflows tab is tapped', () => {
    render(<MobileBottomTabs />)
    fireEvent.click(screen.getByText('Workflows'))
    expect(mockStore.setMainViewMode).toHaveBeenCalledWith('workflows')
  })

  it('closes settings overlay when switching view-mode tabs', () => {
    mockStore.isSettingsOpen = true
    render(<MobileBottomTabs />)
    fireEvent.click(screen.getByText('Workflows'))
    expect(mockStore.setSettingsOpen).toHaveBeenCalledWith(false)
    expect(mockStore.setMainViewMode).toHaveBeenCalledWith('workflows')
  })

  it('toggles Settings via the Settings tab', () => {
    render(<MobileBottomTabs />)
    fireEvent.click(screen.getByText('Settings'))
    expect(mockStore.setSettingsOpen).toHaveBeenCalledWith(true)
    expect(mockStore.setMainViewMode).not.toHaveBeenCalled()
  })

  it('returns null when hidden (virtual keyboard open)', () => {
    const { container } = render(<MobileBottomTabs hidden />)
    expect(container.firstChild).toBeNull()
  })
})
