// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { SessionGroupConfig } from '../src/shared/types'

const mockStore = {
  activeGroupId: null as string | null,
  setActiveGroup: vi.fn(),
  setFocusedTerminal: vi.fn(),
  updateSessionGroup: vi.fn(),
  removeSessionGroup: vi.fn(),
  moveSessionToGroup: vi.fn(),
  terminals: new Map()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

const { SessionGroupItem } =
  await import('../src/renderer/components/project-sidebar/SessionGroupItem')

const group: SessionGroupConfig = {
  id: 'g1',
  name: 'Vorn',
  order: 0,
  workspaceId: 'personal'
}

function renderGroup(overrides: Partial<React.ComponentProps<typeof SessionGroupItem>> = {}) {
  return render(
    <SessionGroupItem
      group={group}
      sessionCount={0}
      hasWaiting={false}
      isExpanded={true}
      onToggleExpanded={vi.fn()}
      {...overrides}
    />
  )
}

beforeEach(() => {
  mockStore.activeGroupId = null
  mockStore.setActiveGroup.mockReset()
  mockStore.setFocusedTerminal.mockReset()
  mockStore.updateSessionGroup.mockReset()
  mockStore.removeSessionGroup.mockReset()
})

describe('a group row is a scope, like All Projects', () => {
  it('selects the group when the row is clicked', () => {
    renderGroup()
    fireEvent.click(screen.getByRole('button', { name: 'Vorn' }))
    expect(mockStore.setActiveGroup).toHaveBeenCalledWith('g1')
    expect(mockStore.setFocusedTerminal).toHaveBeenCalledWith(null)
  })

  it('deselects when the already-selected group is clicked again', () => {
    mockStore.activeGroupId = 'g1'
    renderGroup()
    fireEvent.click(screen.getByRole('button', { name: 'Vorn' }))
    expect(mockStore.setActiveGroup).toHaveBeenCalledWith(null)
  })

  it('reports its selected state', () => {
    mockStore.activeGroupId = 'g1'
    renderGroup()
    expect(screen.getByRole('button', { name: 'Vorn' })).toHaveAttribute('aria-pressed', 'true')
  })
})

describe('the disclosure is separate from the selection', () => {
  it('toggles without selecting the group', () => {
    const onToggleExpanded = vi.fn()
    renderGroup({ onToggleExpanded })
    fireEvent.click(screen.getByRole('button', { name: 'Toggle Vorn' }))
    expect(onToggleExpanded).toHaveBeenCalledTimes(1)
    expect(mockStore.setActiveGroup).not.toHaveBeenCalled()
  })
})

describe('a shut group answers for the sessions it hides', () => {
  it('shows the waiting mark when collapsed', () => {
    renderGroup({ isExpanded: false, hasWaiting: true, sessionCount: 3 })
    expect(screen.getByLabelText('A session in this group is waiting')).toBeInTheDocument()
  })

  it('does not, while the sessions are on screen themselves', () => {
    renderGroup({ isExpanded: true, hasWaiting: true, sessionCount: 3 })
    expect(screen.queryByLabelText('A session in this group is waiting')).not.toBeInTheDocument()
  })

  it('carries the session count', () => {
    renderGroup({ isExpanded: false, sessionCount: 3 })
    expect(screen.getByText('3')).toBeInTheDocument()
  })
})

describe('renaming', () => {
  it('opens straight into the field for a group just created', () => {
    renderGroup({ startRenaming: true })
    expect(screen.getByRole('textbox')).toHaveValue('Vorn')
  })

  it('commits the new name on Enter', () => {
    const onRenameSettled = vi.fn()
    renderGroup({ startRenaming: true, onRenameSettled })
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'Tooling' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(mockStore.updateSessionGroup).toHaveBeenCalledWith('g1', { name: 'Tooling' })
    expect(onRenameSettled).toHaveBeenCalledTimes(1)
  })

  it('keeps the old name on Escape', () => {
    renderGroup({ startRenaming: true })
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'Tooling' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(mockStore.updateSessionGroup).not.toHaveBeenCalled()
  })
})

describe('the group row menu', () => {
  const openMenu = () => {
    renderGroup()
    fireEvent.click(screen.getByRole('button', { name: 'More actions for Vorn' }))
  }

  it('opens from the row', () => {
    openMenu()
    expect(screen.getByText('Rename group')).toBeInTheDocument()
  })

  it('renames through the same inline field', () => {
    openMenu()
    fireEvent.click(screen.getByText('Rename group'))
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'Renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(mockStore.updateSessionGroup).toHaveBeenCalledWith('g1', { name: 'Renamed' })
  })

  it('opens the icon picker, and closes it again', () => {
    openMenu()
    fireEvent.click(screen.getByText('Change icon'))
    expect(screen.getByText('Preview')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Done' }))
    expect(screen.queryByText('Preview')).not.toBeInTheDocument()
  })

  it('writes the icon and colour the picker hands back', () => {
    openMenu()
    fireEvent.click(screen.getByText('Change icon'))
    fireEvent.click(screen.getByRole('button', { name: 'Launch' }))
    expect(mockStore.updateSessionGroup).toHaveBeenCalledWith('g1', { icon: 'Rocket' })
  })

  it('lets every session in the group go, without ending any', () => {
    mockStore.terminals = new Map([
      ['s1', { session: { id: 's1', groupId: 'g1' } }],
      ['s2', { session: { id: 's2', groupId: 'g2' } }],
      ['s3', { session: { id: 's3', groupId: 'g1' } }]
    ]) as never
    openMenu()
    fireEvent.click(screen.getByText('Remove all sessions'))
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s1', null)
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s3', null)
    expect(mockStore.moveSessionToGroup).not.toHaveBeenCalledWith('s2', null)
  })

  it('deletes the group on the second click', () => {
    openMenu()
    fireEvent.click(screen.getByText('Delete group'))
    fireEvent.click(screen.getByText('Confirm delete?'))
    expect(mockStore.removeSessionGroup).toHaveBeenCalledWith('g1')
  })
})
