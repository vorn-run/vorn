import { useMemo } from 'react'
import { useAppStore } from '../stores'
import {
  TaskConfig,
  TaskStatus,
  supportsExactSessionResume,
  getProjectRemoteHostId
} from '../../shared/types'
import { TaskKanbanBoard } from './task-board/TaskKanbanBoard'
import { TaskListView } from './task-board/TaskListView'
import { ListTodo } from 'lucide-react'
import { toast } from './Toast'
import { useWorkspaceProjects } from '../hooks/useWorkspaceProjects'

// The field the whole view sits on, the same rung the workflow canvas uses.
// It was a step higher, which is why tasks read pale next to workflows.
const ROOT_STYLE = { background: 'var(--color-surface-base)' }

export function TaskBoardView() {
  const activeProject = useAppStore((s) => s.activeProject)
  const config = useAppStore((s) => s.config)
  const removeTask = useAppStore((s) => s.removeTask)
  const addTerminal = useAppStore((s) => s.addTerminal)
  const startTask = useAppStore((s) => s.startTask)
  const completeTask = useAppStore((s) => s.completeTask)
  const cancelTask = useAppStore((s) => s.cancelTask)
  const reopenTask = useAppStore((s) => s.reopenTask)
  const updateTask = useAppStore((s) => s.updateTask)
  const terminals = useAppStore((s) => s.terminals)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const setSelectedTaskId = useAppStore((s) => s.setSelectedTaskId)
  const setTaskDialogOpen = useAppStore((s) => s.setTaskDialogOpen)
  const taskStatusFilter = useAppStore((s) => s.taskStatusFilter)
  const taskSourceFilter = useAppStore((s) => s.taskSourceFilter)
  const taskIncludeArchived = useAppStore((s) => s.taskIncludeArchived)
  const archiveTask = useAppStore((s) => s.archiveTask)
  const unarchiveTask = useAppStore((s) => s.unarchiveTask)

  const viewMode = config?.defaults?.taskViewMode ?? 'list'

  const workspaceProjects = useWorkspaceProjects()
  const workspaceProjectNames = useMemo(
    () => new Set(workspaceProjects.map((p) => p.name)),
    [workspaceProjects]
  )

  const projectTasks = (config?.tasks || []).filter((t) =>
    activeProject ? t.projectName === activeProject : workspaceProjectNames.has(t.projectName)
  )

  const visibleTasks = taskIncludeArchived
    ? projectTasks
    : projectTasks.filter((t) => !t.archivedAt)

  // Apply status filter
  const statusFiltered =
    taskStatusFilter === 'all'
      ? visibleTasks
      : visibleTasks.filter((t) => t.status === taskStatusFilter)

  // Apply source filter
  const allTasks =
    taskSourceFilter === 'all'
      ? statusFiltered
      : taskSourceFilter === 'local'
        ? statusFiltered.filter((t) => !t.sourceConnectorId)
        : statusFiltered.filter((t) => t.sourceConnectorId === taskSourceFilter)

  const todoTasks = allTasks.filter((t) => t.status === 'todo').sort((a, b) => a.order - b.order)
  const inProgressTasks = allTasks.filter((t) => t.status === 'in_progress')
  const inReviewTasks = allTasks.filter((t) => t.status === 'in_review')
  const doneTasks = allTasks.filter((t) => t.status === 'done')
  const cancelledTasks = allTasks.filter((t) => t.status === 'cancelled')

  const handleEdit = (task: TaskConfig) => {
    setSelectedTaskId(task.id)
  }

  const getOpenSessionHandler = (task: TaskConfig) => {
    if (task.assignedSessionId && terminals.has(task.assignedSessionId)) {
      return () => setFocusedTerminal(task.assignedSessionId!)
    }
    if (
      task.agentSessionId &&
      task.assignedAgent &&
      supportsExactSessionResume(task.assignedAgent) &&
      (task.status === 'in_progress' || task.status === 'in_review' || task.status === 'done')
    ) {
      return async () => {
        const project = config?.projects.find((p) => p.name === task.projectName)
        const agentType = task.assignedAgent!
        const remoteHostId = project ? getProjectRemoteHostId(project) : undefined
        const session = await window.api.createTerminal({
          agentType,
          projectName: task.projectName,
          projectPath: project?.path || '',
          branch: task.branch,
          useWorktree: task.useWorktree,
          resumeSessionId: task.agentSessionId,
          remoteHostId
        })
        addTerminal(session)
        if (task.status === 'in_progress') {
          startTask(task.id, session.id, agentType)
        }
        setFocusedTerminal(session.id)
      }
    }
    return undefined
  }

  const handleKanbanDrop = (taskId: string, newStatus: TaskStatus) => {
    const task = allTasks.find((t) => t.id === taskId)
    if (!task || task.status === newStatus) return

    if (newStatus === 'in_progress' && task.status === 'todo') {
      // No direct terminal spawn — just transition status and preserve the
      // task's existing `assignedAgent` so the "Default Task Workflow"
      // (or any user workflow with a taskStatusChanged trigger) can read it
      // from context.task at run time. Using `startTask(..., undefined, ...)`
      // here would wipe `assignedAgent`, breaking the "fromTask" resolution
      // that's the whole point of this flow.
      updateTask(taskId, { status: 'in_progress' })
    } else if (newStatus === 'in_review') {
      updateTask(taskId, { status: 'in_review', assignedSessionId: undefined })
    } else if (newStatus === 'done') {
      completeTask(taskId)
      toast.success('Task completed')
    } else if (newStatus === 'cancelled') {
      cancelTask(taskId)
      toast.info('Task cancelled')
    } else if (newStatus === 'todo') {
      reopenTask(taskId)
    }
  }

  const isSessionLive = (task: TaskConfig) =>
    !!(task.assignedSessionId && terminals.has(task.assignedSessionId))

  const handleSelect = (task: TaskConfig) => {
    setSelectedTaskId(task.id)
  }

  const handleAddTask = (status: TaskStatus) => {
    setTaskDialogOpen(true, status)
  }

  const sections: { status: TaskStatus; title: string; tasks: TaskConfig[]; emptyText: string }[] =
    [
      { status: 'todo', title: 'Todo', tasks: todoTasks, emptyText: 'No tasks in queue' },
      {
        status: 'in_progress',
        title: 'In Progress',
        tasks: inProgressTasks,
        emptyText: 'No active tasks'
      },
      {
        status: 'in_review',
        title: 'In Review',
        tasks: inReviewTasks,
        emptyText: 'No tasks awaiting review'
      },
      { status: 'done', title: 'Done', tasks: doneTasks, emptyText: 'No completed tasks' },
      {
        status: 'cancelled',
        title: 'Cancelled',
        tasks: cancelledTasks,
        emptyText: 'No cancelled tasks'
      }
    ]

  const totalTasks = allTasks.length

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0" style={ROOT_STYLE}>
      {/* Content */}
      <div
        className={`flex-1 p-4 ${viewMode === 'kanban' ? 'flex flex-col min-h-0 overflow-hidden' : 'overflow-auto'}`}
      >
        {totalTasks === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center">
            <ListTodo size={40} strokeWidth={1} className="text-ink-faint mb-3" />
            <p className="text-sm text-ink-faint mb-1">
              {taskStatusFilter !== 'all' ? 'No matching tasks' : 'No tasks yet'}
            </p>
            <p className="text-xs text-ink-faint">
              {taskStatusFilter !== 'all'
                ? 'Try changing the status filter'
                : activeProject
                  ? `Create a task for ${activeProject} to get started`
                  : 'Select a project or create a task to get started'}
            </p>
          </div>
        ) : viewMode === 'kanban' ? (
          <TaskKanbanBoard
            allTasks={allTasks}
            onEdit={handleEdit}
            onDelete={(id) => {
              removeTask(id)
              toast.success('Task deleted')
            }}
            onDrop={handleKanbanDrop}
            onOpenSession={getOpenSessionHandler}
            onComplete={(id) => {
              completeTask(id)
              toast.success('Task completed')
            }}
            onCancel={(id) => {
              cancelTask(id)
              toast.info('Task cancelled')
            }}
            onReopen={(id) => {
              reopenTask(id)
              toast.success('Task reopened')
            }}
            onArchive={(id) => {
              archiveTask(id)
              toast.success('Task archived')
            }}
            onUnarchive={(id) => {
              unarchiveTask(id)
              toast.success('Task unarchived')
            }}
            onReviewDiff={(id) => setSelectedTaskId(id)}
            onSelect={handleSelect}
            onAddTask={handleAddTask}
            isSessionLive={isSessionLive}
          />
        ) : (
          <TaskListView
            sections={sections}
            onEdit={handleEdit}
            onDelete={(id) => {
              removeTask(id)
              toast.success('Task deleted')
            }}
            onOpenSession={getOpenSessionHandler}
            onComplete={(id) => {
              completeTask(id)
              toast.success('Task completed')
            }}
            onCancel={(id) => {
              cancelTask(id)
              toast.info('Task cancelled')
            }}
            onReopen={(id) => {
              reopenTask(id)
              toast.success('Task reopened')
            }}
            onArchive={(id) => {
              archiveTask(id)
              toast.success('Task archived')
            }}
            onUnarchive={(id) => {
              unarchiveTask(id)
              toast.success('Task unarchived')
            }}
            onReviewDiff={(id) => setSelectedTaskId(id)}
            onSelect={handleSelect}
            onAddTask={handleAddTask}
            isSessionLive={isSessionLive}
          />
        )}
      </div>
    </div>
  )
}
