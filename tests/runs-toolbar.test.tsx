// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'
import type { RunBucket } from '../src/renderer/stores/types'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))

import { RunsToolbar } from '../src/renderer/components/workflow-runs/RunsToolbar'

const setFilter = vi.fn()

beforeEach(() => {
  setFilter.mockReset()
})

describe('RunsToolbar', () => {
  it('renders the filter button', () => {
    const { container } = render(<RunsToolbar filter="all" setFilter={setFilter} />)
    expect(container.querySelector('button')).toBeInTheDocument()
  })

  it('does not show the active-state indicator when filter is "all"', () => {
    const { container } = render(<RunsToolbar filter="all" setFilter={setFilter} />)
    expect(container.querySelector('.bg-ink-secondary')).not.toBeInTheDocument()
  })

  it('shows the active-state indicator when a non-default filter is active', () => {
    const { container } = render(<RunsToolbar filter="running" setFilter={setFilter} />)
    expect(container.querySelector('.bg-ink-secondary')).toBeInTheDocument()
  })

  it('opens the dropdown with All / Running / Waiting / Failed / Succeeded', () => {
    render(<RunsToolbar filter="all" setFilter={setFilter} />)
    fireEvent.click(screen.getByLabelText('Filter runs'))
    expect(screen.getByText('All')).toBeInTheDocument()
    expect(screen.getByText('Running')).toBeInTheDocument()
    expect(screen.getByText('Waiting')).toBeInTheDocument()
    expect(screen.getByText('Failed')).toBeInTheDocument()
    expect(screen.getByText('Succeeded')).toBeInTheDocument()
  })

  it('calls setFilter with the chosen bucket and closes the dropdown', () => {
    render(<RunsToolbar filter="all" setFilter={setFilter} />)
    fireEvent.click(screen.getByLabelText('Filter runs'))
    fireEvent.click(screen.getByText('Failed'))
    const expected: RunBucket = 'error'
    expect(setFilter).toHaveBeenCalledWith(expected)
    expect(screen.queryByText('Running')).not.toBeInTheDocument()
  })

  it('toggles the dropdown closed when the trigger button is clicked again', () => {
    render(<RunsToolbar filter="all" setFilter={setFilter} />)
    const trigger = screen.getByLabelText('Filter runs')
    fireEvent.click(trigger)
    expect(screen.getByText('Running')).toBeInTheDocument()
    fireEvent.click(trigger)
    expect(screen.queryByText('Running')).not.toBeInTheDocument()
  })
})
