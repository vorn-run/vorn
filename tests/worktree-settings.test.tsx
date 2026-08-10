// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import React from 'react'
import type {
  WorktreeInventory,
  WorktreeInventoryEntry,
  WorktreeProjectInventory
} from '../src/shared/types'

Object.defineProperty(window, 'matchMedia', {
  value: () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn()
  }),
  writable: true
})

vi.mock('framer-motion', () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
  motion: new Proxy(
    {},
    {
      get: (_, tag: string) =>
        React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement>>((props, ref) =>
          React.createElement(tag, { ...props, ref })
        )
    }
  )
}))

vi.mock('../src/renderer/components/Toast', () => ({
  toast: Object.assign((msg: string) => msg, {
    loading: () => 'toast-id',
    update: vi.fn(),
    dismiss: vi.fn(),
    success: vi.fn(),
    error: vi.fn(),
    warning: vi.fn(),
    info: vi.fn()
  })
}))

const mockGetInventory = vi.fn()
const mockReclaim = vi.fn()
const mockRemoveMany = vi.fn()
const mockPrune = vi.fn()
const mockDeleteBranches = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    getWorktreeInventory: (...a: unknown[]) => mockGetInventory(...a),
    reclaimWorktreeArtifacts: (...a: unknown[]) => mockReclaim(...a),
    removeWorktrees: (...a: unknown[]) => mockRemoveMany(...a),
    pruneOrphanWorktrees: (...a: unknown[]) => mockPrune(...a),
    deleteBranches: (...a: unknown[]) => mockDeleteBranches(...a)
  },
  writable: true
})

import { WorktreeSettings } from '../src/renderer/components/settings/WorktreeSettings'

const MB = 1024 ** 2
const GB = 1024 ** 3

function entry(over: Partial<WorktreeInventoryEntry> = {}): WorktreeInventoryEntry {
  return {
    path: '/dev/.vorn-worktrees/vorn/royal-stanza-a0494142',
    name: 'royal-stanza-a0494142',
    projectPath: '/dev/vorn',
    projectName: 'vorn',
    kind: 'registered',
    branch: 'royal-stanza',
    isMain: false,
    sizeBytes: GB,
    artifactBytes: 983 * MB,
    sizeMeasured: true,
    lastCommitAt: '2026-05-07T18:22:48-06:00',
    lastTouchedAt: '2026-05-07T18:22:48-06:00',
    idleDays: 92,
    isDirty: false,
    isMerged: true,
    hasUpstream: true,
    activeSessionIds: [],
    verdict: { level: 'remove', freesBytes: GB, reasons: ['merged'], autoSelect: true },
    ...over
  }
}

function inventory(
  entries: WorktreeInventoryEntry[],
  over: Partial<WorktreeProjectInventory> = {}
) {
  const project: WorktreeProjectInventory = {
    projectPath: '/dev/vorn',
    projectName: 'vorn',
    defaultBranch: 'main',
    remoteHostId: null,
    entries,
    staleBranches: [],
    ...over
  }
  return { projects: [project], scannedAt: '2026-08-07T19:00:00Z' } satisfies WorktreeInventory
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGetInventory.mockResolvedValue(inventory([entry()]))
})

describe('WorktreeSettings', () => {
  it('scans on mount and shows the total on disk', async () => {
    render(<WorktreeSettings />)
    await waitFor(() => expect(screen.getAllByText('1.0 GB').length).toBeGreaterThan(0))
    expect(screen.getByText('On disk')).toBeInTheDocument()
    expect(mockGetInventory).toHaveBeenCalledWith({ refresh: false })
  })

  it('offers the reclaim action sized in bytes, and only acts on the second click', async () => {
    mockReclaim.mockResolvedValue({
      succeeded: ['/dev/.vorn-worktrees/vorn/royal-stanza-a0494142'],
      failed: [],
      freedBytes: 983 * MB,
      deletedBranches: []
    })
    render(<WorktreeSettings />)

    const button = await screen.findByRole('button', { name: /Reclaim 983 MB of build output/ })
    fireEvent.click(button)
    expect(mockReclaim).not.toHaveBeenCalled()

    fireEvent.click(await screen.findByRole('button', { name: /frees 983 MB/ }))
    await waitFor(() =>
      expect(mockReclaim).toHaveBeenCalledWith(['/dev/.vorn-worktrees/vorn/royal-stanza-a0494142'])
    )
  })

  it('will not let a worktree with a live session be selected', async () => {
    mockGetInventory.mockResolvedValue(
      inventory([
        entry({
          activeSessionIds: ['s1'],
          verdict: {
            level: 'keep',
            freesBytes: 0,
            reasons: ['1 active session'],
            autoSelect: false
          }
        })
      ])
    )
    render(<WorktreeSettings />)
    const checkbox = await screen.findByRole('checkbox', { name: /royal-stanza/ })
    expect(checkbox).toBeDisabled()
  })

  it('spells out the byte count before removing, and warns about uncommitted changes', async () => {
    mockGetInventory.mockResolvedValue(
      inventory([
        entry({
          isDirty: true,
          verdict: {
            level: 'review',
            freesBytes: 0,
            reasons: ['uncommitted changes'],
            autoSelect: false
          }
        })
      ])
    )
    mockRemoveMany.mockResolvedValue({
      succeeded: ['/dev/.vorn-worktrees/vorn/royal-stanza-a0494142'],
      failed: [],
      freedBytes: GB,
      deletedBranches: []
    })
    render(<WorktreeSettings />)

    fireEvent.click(await screen.findByRole('checkbox', { name: /royal-stanza/ }))
    expect(await screen.findByText(/uncommitted changes that will be lost/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(mockRemoveMany).not.toHaveBeenCalled()

    fireEvent.click(await screen.findByRole('button', { name: /Yes — remove 1 and free 1\.0 GB/ }))
    await waitFor(() => expect(mockRemoveMany).toHaveBeenCalled())
    // A dirty worktree needs --force, and its branch is not merged so it stays.
    expect(mockRemoveMany.mock.calls[0][0]).toEqual([
      {
        projectPath: '/dev/vorn',
        worktreePath: '/dev/.vorn-worktrees/vorn/royal-stanza-a0494142',
        force: true,
        deleteBranch: true
      }
    ])
  })

  it('routes orphan directories to the prune call, not to git', async () => {
    mockGetInventory.mockResolvedValue(
      inventory([
        entry({
          path: '/dev/.vorn-worktrees/vorn/swift-spark-0f339ef6',
          name: 'swift-spark-0f339ef6',
          kind: 'orphan-dir',
          branch: null,
          sizeBytes: 816 * 1024,
          artifactBytes: 0,
          isMerged: false,
          verdict: {
            level: 'orphan',
            freesBytes: 816 * 1024,
            reasons: ['not registered with git'],
            autoSelect: false
          }
        })
      ])
    )
    mockPrune.mockResolvedValue({
      succeeded: ['/dev/.vorn-worktrees/vorn/swift-spark-0f339ef6'],
      failed: [],
      freedBytes: 816 * 1024,
      deletedBranches: []
    })
    render(<WorktreeSettings />)

    fireEvent.click(await screen.findByRole('checkbox', { name: /swift-spark/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    fireEvent.click(await screen.findByRole('button', { name: /Yes — remove 1/ }))

    await waitFor(() =>
      expect(mockPrune).toHaveBeenCalledWith(['/dev/.vorn-worktrees/vorn/swift-spark-0f339ef6'])
    )
    expect(mockRemoveMany).not.toHaveBeenCalled()
  })

  it('lists branches left behind and deletes only the ones picked', async () => {
    mockGetInventory.mockResolvedValue(
      inventory([entry()], {
        staleBranches: [
          { name: 'ivory-relic', isMerged: true, hasUpstream: false, lastCommitAt: null },
          { name: 'sienna-etching', isMerged: false, hasUpstream: true, lastCommitAt: null }
        ]
      })
    )
    mockDeleteBranches.mockResolvedValue({ deleted: ['ivory-relic'], failed: [] })
    render(<WorktreeSettings />)

    expect(
      await screen.findByText(/2 branches left behind by removed worktrees — 1 already merged/)
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Show' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Select 1 merged' }))
    fireEvent.click(screen.getByRole('button', { name: /Delete 1 branch/ }))

    await waitFor(() =>
      expect(mockDeleteBranches).toHaveBeenCalledWith('/dev/vorn', ['ivory-relic'], false)
    )
  })

  it('excludes the main worktree from the list entirely', async () => {
    mockGetInventory.mockResolvedValue(
      inventory([
        entry({ path: '/dev/vorn', name: 'vorn', isMain: true, sizeBytes: 0, artifactBytes: 0 }),
        entry()
      ])
    )
    render(<WorktreeSettings />)
    await screen.findByRole('checkbox', { name: /royal-stanza/ })
    expect(screen.queryByRole('checkbox', { name: 'Select vorn' })).not.toBeInTheDocument()
  })

  it('says so plainly when there is nothing to clean up', async () => {
    mockGetInventory.mockResolvedValue(inventory([]))
    render(<WorktreeSettings />)
    expect(await screen.findByText(/No worktrees yet/)).toBeInTheDocument()
  })
})
