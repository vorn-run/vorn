// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import type { UpdateStatus } from '../src/shared/types'

const mockStore = {
  setSettingsOpen: vi.fn(),
  setSettingsCategory: vi.fn(),
  setOnboardingOpen: vi.fn(),
  appUpdateStatus: { kind: 'unsupported' } as UpdateStatus,
  updateBannerDismissed: false,
  setUpdateBannerDismissed: vi.fn(),
  /** The banner says what restarting costs, so it reads the board. */
  terminals: new Map<string, { status: string }>()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockStore) : mockStore
  }
}))

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>
}))

const installUpdate = vi.fn()

Object.defineProperty(window, 'api', {
  value: { installUpdate },
  writable: true
})

const { SidebarFooter } = await import('../src/renderer/components/project-sidebar/SidebarFooter')

beforeEach(() => {
  mockStore.setSettingsOpen.mockReset()
  mockStore.setSettingsCategory.mockReset()
  mockStore.setOnboardingOpen.mockReset()
  mockStore.setUpdateBannerDismissed.mockReset()
  installUpdate.mockReset()
  mockStore.appUpdateStatus = { kind: 'unsupported' }
  mockStore.updateBannerDismissed = false
  mockStore.terminals = new Map()
})

describe('SidebarFooter', () => {
  it('renders Welcome Guide and Settings icon buttons with accessible names', () => {
    render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Welcome Guide' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()
  })

  it('opens the Welcome Guide when the help button is clicked', () => {
    render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Welcome Guide' }))
    expect(mockStore.setOnboardingOpen).toHaveBeenCalledWith(true)
  })

  it('opens Settings and dismisses the mobile sidebar when the settings button is clicked', () => {
    const closeSidebarOnMobile = vi.fn()
    render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={closeSidebarOnMobile} />)
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(mockStore.setSettingsOpen).toHaveBeenCalledWith(true)
    expect(closeSidebarOnMobile).toHaveBeenCalled()
  })

  it('still renders both buttons when the sidebar is collapsed', () => {
    render(<SidebarFooter isCollapsed={true} closeSidebarOnMobile={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Welcome Guide' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()
  })

  describe('update affordance', () => {
    it('shows nothing when no update is staged', () => {
      mockStore.appUpdateStatus = { kind: 'checking' }
      render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)
      expect(screen.queryByText(/ready/i)).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()
    })

    it('shows the banner with a restart action once an update is ready', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.6.0' }
      render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)

      expect(screen.getByText('v0.6.0 ready')).toBeInTheDocument()
      fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }))
      expect(installUpdate).toHaveBeenCalled()
    })

    it('falls back to a badged gear when collapsed, since the banner cannot fit', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.6.0' }
      render(<SidebarFooter isCollapsed={true} closeSidebarOnMobile={vi.fn()} />)

      expect(screen.queryByText('v0.6.0 ready')).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Settings, update ready' })).toBeInTheDocument()
    })

    it('keeps the gear badged after dismissal, so the update stays findable', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.6.0' }
      mockStore.updateBannerDismissed = true
      render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)

      expect(screen.queryByText('v0.6.0 ready')).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Settings, update ready' })).toBeInTheDocument()
    })

    it('dismisses the banner without clearing the staged update', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.6.0' }
      render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)

      fireEvent.click(screen.getByRole('button', { name: 'Dismiss update notice' }))
      expect(mockStore.setUpdateBannerDismissed).toHaveBeenCalledWith(true)
    })

    it('deep-links the badged gear to the Updates panel rather than the last category', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.6.0' }
      mockStore.updateBannerDismissed = true
      render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)

      fireEvent.click(screen.getByRole('button', { name: 'Settings, update ready' }))
      expect(mockStore.setSettingsCategory).toHaveBeenCalledWith('updates')
      expect(mockStore.setSettingsOpen).toHaveBeenCalledWith(true)
    })
  })
})

describe('what the sidebar restart will cost', () => {
  it('says it here too, because this button ends the sessions as well', () => {
    // The shortcut must not be the quieter way to do the same thing.
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([
      ['a', { status: 'running' }],
      ['b', { status: 'idle' }]
    ])
    render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)
    expect(screen.getByText(/Your 2 sessions restart on the new version/)).toBeInTheDocument()
    expect(screen.getByText(/A turn in flight is lost/)).toBeInTheDocument()
  })

  it('says nothing when there are no sessions to lose', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    render(<SidebarFooter isCollapsed={false} closeSidebarOnMobile={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Restart to update' })).toBeInTheDocument()
    expect(screen.queryByText(/restart on the new version/)).not.toBeInTheDocument()
  })
})
