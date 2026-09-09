// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { SessionGroupConfig } from '../src/shared/types'

const mockStore = {
  moveSessionToGroup: vi.fn(),
  activeWorkspace: 'personal',
  config: {
    sessionGroups: [
      { id: 'g1', name: 'Sidebar work', order: 0, workspaceId: 'personal', icon: 'Terminal' },
      { id: 'g2', name: 'Release', order: 1, workspaceId: 'personal' },
      { id: 'g9', name: 'Elsewhere', order: 0, workspaceId: 'work' }
    ]
  }
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const toastSuccess = vi.fn()
vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: toastSuccess, error: vi.fn() }
}))

const { GroupContextMenu } =
  await import('../src/renderer/components/project-sidebar/GroupContextMenu')
const { SessionContextMenu } =
  await import('../src/renderer/components/project-sidebar/SessionContextMenu')

const group: SessionGroupConfig = {
  id: 'g1',
  name: 'Sidebar work',
  order: 0,
  workspaceId: 'personal'
}

beforeEach(() => vi.clearAllMocks())

describe('the group menu', () => {
  const handlers = () => ({
    onRename: vi.fn(),
    onChangeIcon: vi.fn(),
    onUngroup: vi.fn(),
    onDelete: vi.fn(),
    onClose: vi.fn()
  })

  it('offers rename, icon, ungroup and delete', () => {
    render(<GroupContextMenu group={group} {...handlers()} />)
    expect(screen.getByText('Rename group')).toBeInTheDocument()
    expect(screen.getByText('Change icon')).toBeInTheDocument()
    expect(screen.getByText('Remove all sessions')).toBeInTheDocument()
    expect(screen.getByText('Delete group')).toBeInTheDocument()
  })

  it.each([
    ['Rename group', 'onRename'],
    ['Change icon', 'onChangeIcon'],
    ['Remove all sessions', 'onUngroup']
  ])('%s calls %s and closes', (label, handler) => {
    const h = handlers()
    render(<GroupContextMenu group={group} {...h} />)
    fireEvent.click(screen.getByText(label))
    expect(h[handler as keyof typeof h]).toHaveBeenCalledTimes(1)
    expect(h.onClose).toHaveBeenCalledTimes(1)
  })

  /** Delete is two clicks so a slip cannot take a group with it. */
  it('does not delete on the first click', () => {
    const h = handlers()
    render(<GroupContextMenu group={group} {...h} />)
    fireEvent.click(screen.getByText('Delete group'))
    expect(h.onDelete).not.toHaveBeenCalled()
    expect(screen.getByText('Confirm delete?')).toBeInTheDocument()
  })

  it('deletes on the second', () => {
    const h = handlers()
    render(<GroupContextMenu group={group} {...h} />)
    fireEvent.click(screen.getByText('Delete group'))
    fireEvent.click(screen.getByText('Confirm delete?'))
    expect(h.onDelete).toHaveBeenCalledTimes(1)
    expect(toastSuccess).toHaveBeenCalledWith('Group "Sidebar work" deleted')
  })

  it('closes when a click lands outside it', () => {
    const h = handlers()
    render(<GroupContextMenu group={group} {...h} />)
    fireEvent.pointerDown(document.body)
    expect(h.onClose).toHaveBeenCalled()
  })
})

describe('the session menu', () => {
  it('lists only the groups of the active workspace', () => {
    render(<SessionContextMenu sessionId="s1" onNewGroup={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByText('Sidebar work')).toBeInTheDocument()
    expect(screen.getByText('Release')).toBeInTheDocument()
    expect(screen.queryByText('Elsewhere')).not.toBeInTheDocument()
  })

  it('files the session into the group that was picked', () => {
    const onClose = vi.fn()
    render(<SessionContextMenu sessionId="s1" onNewGroup={vi.fn()} onClose={onClose} />)
    fireEvent.click(screen.getByText('Release'))
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s1', 'g2')
    expect(onClose).toHaveBeenCalled()
  })

  it('takes it out again when its current group is picked', () => {
    render(
      <SessionContextMenu
        sessionId="s1"
        currentGroupId="g1"
        onNewGroup={vi.fn()}
        onClose={vi.fn()}
      />
    )
    fireEvent.click(screen.getByText('Sidebar work'))
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s1', null)
  })

  it('offers Remove from group only while it is in one', () => {
    const { unmount } = render(
      <SessionContextMenu sessionId="s1" onNewGroup={vi.fn()} onClose={vi.fn()} />
    )
    expect(screen.queryByText('Remove from group')).not.toBeInTheDocument()
    unmount()

    render(
      <SessionContextMenu
        sessionId="s1"
        currentGroupId="g1"
        onNewGroup={vi.fn()}
        onClose={vi.fn()}
      />
    )
    fireEvent.click(screen.getByText('Remove from group'))
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s1', null)
  })

  it('can make a new group instead', () => {
    const onNewGroup = vi.fn()
    render(<SessionContextMenu sessionId="s1" onNewGroup={onNewGroup} onClose={vi.fn()} />)
    fireEvent.click(screen.getByText('New group…'))
    expect(onNewGroup).toHaveBeenCalledTimes(1)
  })

  it('closes when a click lands outside it', () => {
    const onClose = vi.fn()
    render(<SessionContextMenu sessionId="s1" onNewGroup={vi.fn()} onClose={onClose} />)
    fireEvent.pointerDown(document.body)
    expect(onClose).toHaveBeenCalled()
  })
})
