import { describe, it, expect, beforeEach, vi } from 'vitest'

vi.mock('../src/renderer/lib/preferences', () => ({
  loadGridSettings: () => ({}),
  loadSidebarSettings: () => ({}),
  loadFlexibleLayouts: () => ({})
}))

const saveConfig = vi.fn()
;(global as unknown as { window: object }).window = {
  api: { saveConfig }
}

const { createUISlice } = await import('../src/renderer/stores/ui-slice')

interface MinimalState {
  config: {
    workflows: { id: string; name: string; workspaceId?: string }[]
  } | null
  activeWorkspace: string
  sidebarWorkflowFilter: 'all' | 'manual' | 'scheduled'
}

function makeSlice() {
  let state: MinimalState & Record<string, unknown> = {} as MinimalState & Record<string, unknown>
  const set = (updater: ((s: MinimalState) => Partial<MinimalState>) | Partial<MinimalState>) => {
    const patch = typeof updater === 'function' ? updater(state) : updater
    state = { ...state, ...patch }
  }
  const get = () => state as MinimalState
  const slice = createUISlice(set as never, get as never, {} as never)
  Object.assign(state, slice)
  return { state: () => state, slice }
}

describe('reorderWorkflows store action', () => {
  beforeEach(() => {
    saveConfig.mockReset()
  })

  it('moves a workflow within the active workspace', () => {
    const { state, slice } = makeSlice()
    state().config = {
      workflows: [
        { id: 'a', name: 'A', workspaceId: 'personal' },
        { id: 'b', name: 'B', workspaceId: 'personal' },
        { id: 'c', name: 'C', workspaceId: 'personal' }
      ]
    }
    state().activeWorkspace = 'personal'
    slice.reorderWorkflows(0, 2)
    expect(state().config!.workflows.map((w) => w.id)).toEqual(['b', 'c', 'a'])
    expect(saveConfig).toHaveBeenCalledTimes(1)
  })

  it('does nothing when fromIndex equals toIndex', () => {
    const { state, slice } = makeSlice()
    state().config = {
      workflows: [
        { id: 'a', name: 'A', workspaceId: 'personal' },
        { id: 'b', name: 'B', workspaceId: 'personal' }
      ]
    }
    state().activeWorkspace = 'personal'
    slice.reorderWorkflows(1, 1)
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('preserves workflows from other workspaces', () => {
    const { state, slice } = makeSlice()
    state().config = {
      workflows: [
        { id: 'x', name: 'X', workspaceId: 'team-a' },
        { id: 'a', name: 'A', workspaceId: 'personal' },
        { id: 'b', name: 'B', workspaceId: 'personal' },
        { id: 'y', name: 'Y', workspaceId: 'team-a' }
      ]
    }
    state().activeWorkspace = 'personal'
    slice.reorderWorkflows(0, 1)
    const ids = state().config!.workflows.map((w) => w.id)
    expect(ids).toEqual(['x', 'b', 'a', 'y'])
  })

  it('returns early when there is no config', () => {
    const { state, slice } = makeSlice()
    state().config = null
    state().activeWorkspace = 'personal'
    slice.reorderWorkflows(0, 1)
    expect(saveConfig).not.toHaveBeenCalled()
  })
})

describe('setSidebarWorkflowFilter store action', () => {
  beforeEach(() => {
    saveConfig.mockReset()
  })

  it('updates the filter value', () => {
    const { state, slice } = makeSlice()
    slice.setSidebarWorkflowFilter('scheduled')
    expect(state().sidebarWorkflowFilter).toBe('scheduled')
  })
})
