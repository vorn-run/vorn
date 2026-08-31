// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children, label }: { children: ReactNode; label: string }) => (
    <span data-tooltip-label={label}>{children}</span>
  )
}))

const waitingMock = vi.fn(() => [] as unknown[])
vi.mock('../src/renderer/hooks/useWaitingApprovals', () => ({
  useWaitingApprovals: () => waitingMock()
}))

Object.defineProperty(window, 'api', {
  value: { detectIDEs: vi.fn().mockResolvedValue([]) },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { MainViewPills } from '../src/renderer/components/MainViewPills'

const setMainViewMode = vi.fn()

beforeEach(() => {
  setMainViewMode.mockClear()
  waitingMock.mockReset()
  waitingMock.mockReturnValue([])
  useAppStore.setState({
    config: {
      version: 1,
      defaults: {
        shell: 'bash',
        fontSize: 14,
        theme: 'dark' as const,
        mainViewMode: 'sessions' as const
      },
      projects: []
    },
    setMainViewMode
  })
})

describe('MainViewPills', () => {
  it('renders Sessions, Tasks, and Workflows buttons', () => {
    render(<MainViewPills />)
    expect(screen.getByRole('button', { name: 'Sessions' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Tasks' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Workflows' })).toBeInTheDocument()
  })

  it('marks the active view mode with aria-pressed', () => {
    render(<MainViewPills />)
    expect(screen.getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Tasks' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('calls setMainViewMode when a pill is clicked', () => {
    render(<MainViewPills />)
    fireEvent.click(screen.getByRole('button', { name: 'Tasks' }))
    expect(setMainViewMode).toHaveBeenCalledWith('tasks')
  })

  it('applies active styling to the current view mode', () => {
    render(<MainViewPills />)
    expect(screen.getByRole('button', { name: 'Sessions' }).className).toContain('text-white')
    expect(screen.getByRole('button', { name: 'Workflows' }).className).toContain('text-gray-500')
  })

  it('renders no waiting badge when no approvals are pending', () => {
    const { container } = render(<MainViewPills />)
    expect(container.querySelector('.bg-amber-400')).not.toBeInTheDocument()
  })

  it('renders the waiting count on the Workflows pill when approvals are pending', () => {
    waitingMock.mockReturnValue([{}, {}, {}])
    render(<MainViewPills />)
    expect(screen.getByText('3')).toBeInTheDocument()
    const tooltip = screen.getByText('3').closest('[data-tooltip-label]')
    expect(tooltip?.getAttribute('data-tooltip-label')).toContain('3 awaiting review')
  })

  it('caps the waiting badge at "9+"', () => {
    waitingMock.mockReturnValue(new Array(15).fill({}))
    render(<MainViewPills />)
    expect(screen.getByText('9+')).toBeInTheDocument()
  })
})
