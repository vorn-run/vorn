// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../packages/web/src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))

let lastIsOpen = false
vi.mock('../packages/web/src/renderer/components/RecentSessionsPopover', () => ({
  RecentSessionsPopover: ({ isOpen }: { isOpen: boolean }) => {
    lastIsOpen = isOpen
    return isOpen ? <div data-testid="popover">popover</div> : null
  }
}))

Object.defineProperty(window, 'api', {
  value: { detectIDEs: vi.fn().mockResolvedValue([]) },
  writable: true
})

import { RecentSessionsButton } from '../packages/web/src/renderer/components/RecentSessionsButton'

describe('RecentSessionsButton', () => {
  it('renders a button with accessible label', () => {
    render(<RecentSessionsButton />)
    expect(screen.getByRole('button', { name: 'Recent sessions' })).toBeInTheDocument()
  })

  it('toggles popover open/closed on click', () => {
    render(<RecentSessionsButton />)
    expect(lastIsOpen).toBe(false)

    fireEvent.click(screen.getByRole('button', { name: 'Recent sessions' }))
    expect(screen.getByTestId('popover')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Recent sessions' }))
    expect(screen.queryByTestId('popover')).not.toBeInTheDocument()
  })
})
