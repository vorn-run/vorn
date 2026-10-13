// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../packages/web/src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))

Object.defineProperty(window, 'api', {
  value: { detectIDEs: vi.fn().mockResolvedValue([]) },
  writable: true
})

import { useAppStore } from '../packages/web/src/renderer/stores'
import { SidebarToggleButton } from '../packages/web/src/renderer/components/SidebarToggleButton'

const toggleSidebar = vi.fn()

beforeEach(() => {
  toggleSidebar.mockClear()
  useAppStore.setState({ toggleSidebar })
})

describe('SidebarToggleButton', () => {
  it('renders a button with accessible label', () => {
    render(<SidebarToggleButton />)
    expect(screen.getByRole('button', { name: 'Toggle sidebar' })).toBeInTheDocument()
  })

  it('calls toggleSidebar on click', () => {
    render(<SidebarToggleButton />)
    fireEvent.click(screen.getByRole('button', { name: 'Toggle sidebar' }))
    expect(toggleSidebar).toHaveBeenCalledTimes(1)
  })
})
