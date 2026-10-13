// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Minimal store state the toolbar needs.
const state = {
  taskStatusFilter: 'all',
  taskSourceFilter: 'all',
  setTaskStatusFilter: vi.fn(),
  setTaskSourceFilter: vi.fn(),
  config: {
    defaults: { taskViewMode: 'list' },
    tasks: [
      { id: 't1', sourceConnectorId: 'github' },
      { id: 't2', sourceConnectorId: 'linear' },
      { id: 't3' }
    ]
  },
  setConfig: vi.fn()
}

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (s: typeof state) => unknown) => (selector ? selector(state) : state)
}))

const listConnectorsMock = vi.fn()

beforeEach(() => {
  listConnectorsMock.mockReset()
  listConnectorsMock.mockResolvedValue([
    { id: 'github', name: 'GitHub', icon: 'github', capabilities: [], manifest: { auth: [] } },
    { id: 'linear', name: 'Linear', icon: 'linear', capabilities: [], manifest: { auth: [] } }
  ])
  Object.defineProperty(window, 'api', {
    configurable: true,
    value: {
      listConnectors: listConnectorsMock,
      saveConfig: vi.fn()
    }
  })
  // Fresh spies each test so call counts don't leak.
  state.setTaskSourceFilter = vi.fn()
  state.setTaskStatusFilter = vi.fn()
  state.taskSourceFilter = 'all'
})

import { TaskToolbar } from '../packages/web/src/renderer/components/TaskToolbar'

describe('TaskToolbar source filter', () => {
  it('renders the view-options button', () => {
    const { container } = render(<TaskToolbar />)
    expect(container.querySelector('button')).toBeInTheDocument()
  })

  it('opens the dropdown on click and fetches connector names', async () => {
    const { container, getByText } = render(<TaskToolbar />)
    fireEvent.click(container.querySelector('button')!)
    await waitFor(() => {
      expect(listConnectorsMock).toHaveBeenCalled()
    })
    // Once the fetch resolves, brand names surface correctly.
    await waitFor(() => {
      expect(getByText('GitHub')).toBeInTheDocument()
      expect(getByText('Linear')).toBeInTheDocument()
    })
  })

  it('surfaces "All Sources" and "Local Only" options when connectors exist', async () => {
    const { container, getByText } = render(<TaskToolbar />)
    fireEvent.click(container.querySelector('button')!)
    await waitFor(() => expect(getByText('All Sources')).toBeInTheDocument())
    expect(getByText('Local Only')).toBeInTheDocument()
  })

  it('invokes setTaskSourceFilter when a connector option is clicked', async () => {
    const { container, getByText } = render(<TaskToolbar />)
    fireEvent.click(container.querySelector('button')!)
    await waitFor(() => expect(getByText('GitHub')).toBeInTheDocument())
    fireEvent.click(getByText('GitHub'))
    expect(state.setTaskSourceFilter).toHaveBeenCalledWith('github')
  })

  it('invokes setTaskSourceFilter with "local" when Local Only is clicked', async () => {
    const { container, getByText } = render(<TaskToolbar />)
    fireEvent.click(container.querySelector('button')!)
    await waitFor(() => expect(getByText('Local Only')).toBeInTheDocument())
    fireEvent.click(getByText('Local Only'))
    expect(state.setTaskSourceFilter).toHaveBeenCalledWith('local')
  })

  it('falls back to a naive-caps label when the manifest name is missing', async () => {
    listConnectorsMock.mockResolvedValue([]) // no manifests → fallback
    const { container, getByText } = render(<TaskToolbar />)
    fireEvent.click(container.querySelector('button')!)
    await waitFor(() => expect(getByText('Github')).toBeInTheDocument())
    expect(getByText('Linear')).toBeInTheDocument()
  })
})
