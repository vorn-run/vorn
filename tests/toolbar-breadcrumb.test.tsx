// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

Object.defineProperty(window, 'api', {
  value: {
    listBranches: vi.fn().mockResolvedValue({ local: ['main', 'feat'], remote: [] }),
    listRemoteBranches: vi.fn().mockResolvedValue([]),
    checkoutBranch: vi.fn().mockResolvedValue({ ok: true })
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { ToolbarBreadcrumb } from '../src/renderer/components/ToolbarBreadcrumb'

const initialState = useAppStore.getState()

beforeEach(() => {
  act(() => {
    useAppStore.setState({
      activeProject: 'vorn',
      activeWorktreePath: '/tmp/vorn/wt/feat',
      worktreeCache: new Map([
        [
          '/tmp/vorn',
          [
            {
              path: '/tmp/vorn/wt/feat',
              name: 'feat',
              branch: 'feat',
              isMain: false
            }
          ]
        ]
      ]),
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      config: { projects: [{ name: 'vorn', path: '/tmp/vorn' }] } as any
    })
  })
})

afterEach(() => {
  act(() => {
    useAppStore.setState(initialState)
  })
})

describe('ToolbarBreadcrumb', () => {
  it('renders project, worktree, branch with interactive picker', async () => {
    render(<ToolbarBreadcrumb />)
    expect(screen.getByText('vorn')).toBeInTheDocument()
    expect(screen.getAllByText('feat').length).toBeGreaterThan(0)
  })

  it('opens BranchPicker and invokes checkoutBranch on select', async () => {
    render(<ToolbarBreadcrumb />)
    const branchButton = screen.getByRole('button', { name: /feat/ })
    fireEvent.click(branchButton)
    const mainEntry = await screen.findByText('main')
    fireEvent.click(mainEntry)
    await waitFor(() =>
      expect(window.api.checkoutBranch).toHaveBeenCalledWith('/tmp/vorn/wt/feat', 'main')
    )
  })

  it('closes picker when BranchPicker requests onClose (Escape)', async () => {
    render(<ToolbarBreadcrumb />)
    const branchButton = screen.getByRole('button', { name: /feat/ })
    fireEvent.click(branchButton)
    await screen.findByText('main')
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByText('main')).not.toBeInTheDocument())
  })
})

/**
 * The crumb is the only thing that names the current scope, so it must agree
 * with what the scope actually is — including when the selection is stale.
 */
describe('the crumb for a group', () => {
  const groups = [
    { id: 'g1', name: 'Sidebar work', order: 0, workspaceId: 'personal' },
    { id: 'g9', name: 'Elsewhere', order: 0, workspaceId: 'work' }
  ]

  const seed = (activeGroupId: string | null) =>
    act(() => {
      useAppStore.setState({
        activeProject: null,
        activeWorktreePath: null,
        activeGroupId,
        activeWorkspace: 'personal',
        config: { projects: [], sessionGroups: groups }
      } as never)
    })

  afterEach(() => {
    act(() => {
      useAppStore.setState(initialState)
    })
  })

  it('names the selected group', () => {
    seed('g1')
    render(<ToolbarBreadcrumb />)
    expect(screen.getByText('Sidebar work')).toBeInTheDocument()
  })

  it('clears the selection when the crumb is clicked', () => {
    seed('g1')
    render(<ToolbarBreadcrumb />)
    fireEvent.click(screen.getByText('Sidebar work'))
    expect(useAppStore.getState().activeGroupId).toBeNull()
  })

  it('says nothing for a group belonging to another workspace', () => {
    seed('g9')
    const { container } = render(<ToolbarBreadcrumb />)
    expect(container).toBeEmptyDOMElement()
  })

  it('says nothing when nothing is selected', () => {
    seed(null)
    const { container } = render(<ToolbarBreadcrumb />)
    expect(container).toBeEmptyDOMElement()
  })
})
