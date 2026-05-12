// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { TaskConfig, AppConfig } from '../src/shared/types'

// Heavy / unrelated children are mocked so the test focuses on the
// refactored state-init machinery (form key, related-runs reset, image-path
// effect, diff-key reset).

vi.mock('../src/renderer/components/rich-editor/RichMarkdownEditor', () => ({
  RichMarkdownEditor: ({ value }: { value: string }) => (
    <textarea data-testid="rich-editor" defaultValue={value} />
  )
}))
vi.mock('../src/renderer/components/AgentPicker', () => ({
  AgentPicker: () => <div data-testid="agent-picker" />
}))
vi.mock('../src/renderer/components/DiffSidebar', () => ({
  DiffFileList: () => null,
  DiffContent: () => null
}))
vi.mock('../src/renderer/components/CommitDialog', () => ({ CommitDialog: () => null }))
vi.mock('../src/renderer/components/StatusPicker', () => ({ StatusPicker: () => null }))
vi.mock('../src/renderer/components/ProjectPicker', () => ({ ProjectPicker: () => null }))
vi.mock('../src/renderer/components/ConnectorIcon', () => ({ ConnectorIcon: () => null }))
vi.mock('../src/renderer/components/workflow-editor/RunEntry', () => ({ RunEntry: () => null }))
vi.mock('../src/renderer/components/LogReplayModal', () => ({ LogReplayModal: () => null }))
vi.mock('../src/renderer/components/ConfirmPopover', () => ({ ConfirmPopover: () => null }))
vi.mock('../src/renderer/components/Toast', () => ({ toast: { success: vi.fn(), info: vi.fn() } }))
vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({ status: {} })
}))

import { useAppStore } from '../src/renderer/stores'
import { TaskDetailPanel } from '../src/renderer/components/TaskDetailPanel'
import { TASK_TEMPLATE } from '../src/renderer/components/MarkdownEditor'

const initialState = useAppStore.getState()

const taskA: TaskConfig = {
  id: 'task-a',
  projectName: 'demo',
  title: 'Alpha task',
  description: 'Alpha description',
  status: 'todo',
  order: 0,
  branch: 'feat/alpha',
  useWorktree: true,
  assignedAgent: 'claude'
}

const taskB: TaskConfig = {
  id: 'task-b',
  projectName: 'demo',
  title: 'Beta task',
  description: 'Beta description',
  status: 'todo',
  order: 1,
  images: ['cover.png']
}

const baseConfig: AppConfig = {
  ...(initialState.config as AppConfig),
  projects: [{ name: 'demo', path: '/demo' }],
  tasks: [taskA, taskB],
  workflows: []
} as AppConfig

const listRuns = vi.fn().mockResolvedValue([])
const getTaskImagePath = vi.fn().mockResolvedValue('/abs/cover.png')
const getGitDiffFull = vi.fn().mockResolvedValue({ files: [], diff: '' })

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const api: any = {
  listWorkflowRunsByTask: listRuns,
  getTaskImagePath,
  getGitDiffFull
}
;(globalThis as unknown as { window: { api: typeof api } }).window =
  (globalThis as unknown as { window?: { api: typeof api } }).window || ({ api } as never)
;(globalThis as { window: { api: typeof api } }).window.api = api

describe('TaskDetailPanel form initialization', () => {
  beforeEach(() => {
    listRuns.mockClear()
    getTaskImagePath.mockClear()
    getGitDiffFull.mockClear()
    useAppStore.setState({
      ...initialState,
      config: baseConfig,
      activeProject: 'demo',
      selectedTaskId: null,
      workflowExecutions: new Map()
    })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('returns null when no task is selected and not in create mode', () => {
    const { container } = render(<TaskDetailPanel />)
    expect(container.firstChild).toBeNull()
  })

  it('populates the form from the selected task', async () => {
    useAppStore.setState({ selectedTaskId: taskA.id })
    render(<TaskDetailPanel />)
    expect(await screen.findByDisplayValue('Alpha task')).toBeInTheDocument()
    expect(screen.getByDisplayValue('feat/alpha')).toBeInTheDocument()
    await waitFor(() => expect(listRuns).toHaveBeenCalledWith(taskA.id, 20))
  })

  it('seeds the create-mode form with the task template and active project', async () => {
    useAppStore.setState({ selectedTaskId: 'new' })
    render(<TaskDetailPanel />)
    await waitFor(() => expect(screen.getByTestId('rich-editor')).toHaveValue(TASK_TEMPLATE))
  })

  it('re-initializes the form when switching between two existing tasks', async () => {
    useAppStore.setState({ selectedTaskId: taskA.id })
    const { rerender } = render(<TaskDetailPanel />)
    expect(await screen.findByDisplayValue('Alpha task')).toBeInTheDocument()

    await act(async () => {
      useAppStore.setState({ selectedTaskId: taskB.id })
    })
    rerender(<TaskDetailPanel />)

    expect(await screen.findByDisplayValue('Beta task')).toBeInTheDocument()
    expect(screen.queryByDisplayValue('Alpha task')).not.toBeInTheDocument()
  })

  it('resolves image paths for tasks that have images', async () => {
    useAppStore.setState({ selectedTaskId: taskB.id })
    render(<TaskDetailPanel />)
    await waitFor(() => expect(getTaskImagePath).toHaveBeenCalledWith(taskB.id, 'cover.png'))
  })

  it('does not call listWorkflowRunsByTask for create mode', async () => {
    useAppStore.setState({ selectedTaskId: 'new' })
    render(<TaskDetailPanel />)
    await waitFor(() => expect(screen.getByTestId('rich-editor')).toBeInTheDocument())
    expect(listRuns).not.toHaveBeenCalled()
  })

  it('re-queries workflow runs when a related execution arrives in the store', async () => {
    useAppStore.setState({ selectedTaskId: taskA.id })
    render(<TaskDetailPanel />)
    await waitFor(() => expect(listRuns).toHaveBeenCalledWith(taskA.id, 20))
    listRuns.mockClear()

    await act(async () => {
      useAppStore.setState({
        workflowExecutions: new Map([
          [
            'exec-1',
            {
              triggerTaskId: taskA.id,
              nodeStates: []
            } as never
          ]
        ])
      })
    })

    await waitFor(() => expect(listRuns).toHaveBeenCalledWith(taskA.id, 20))
  })

  it('fetches the diff for in_review tasks', async () => {
    const reviewTask: TaskConfig = { ...taskA, id: 'task-review', status: 'in_review' }
    useAppStore.setState({
      config: { ...baseConfig, tasks: [reviewTask] } as AppConfig,
      selectedTaskId: reviewTask.id
    })
    render(<TaskDetailPanel />)
    await waitFor(() => expect(getGitDiffFull).toHaveBeenCalledWith('/demo'))
  })

  it('clears diff state when switching from a review task to a non-review task', async () => {
    const reviewTask: TaskConfig = { ...taskA, id: 'task-review', status: 'in_review' }
    const todoTask: TaskConfig = { ...taskB, status: 'todo' }
    useAppStore.setState({
      config: { ...baseConfig, tasks: [reviewTask, todoTask] } as AppConfig,
      selectedTaskId: reviewTask.id
    })
    const { rerender } = render(<TaskDetailPanel />)
    await waitFor(() => expect(getGitDiffFull).toHaveBeenCalled())
    getGitDiffFull.mockClear()

    await act(async () => {
      useAppStore.setState({ selectedTaskId: todoTask.id })
    })
    rerender(<TaskDetailPanel />)

    expect(await screen.findByDisplayValue('Beta task')).toBeInTheDocument()
    expect(getGitDiffFull).not.toHaveBeenCalled()
  })
})
