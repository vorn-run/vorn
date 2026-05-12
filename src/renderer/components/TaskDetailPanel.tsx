import { useState, useEffect, useLayoutEffect, useCallback, useRef, Suspense, lazy } from 'react'
import { useAppStore } from '../stores'
import {
  AiAgentType,
  GitDiffResult,
  WorkflowExecution,
  SessionLog,
  supportsExactSessionResume,
  getProjectRemoteHostId,
  isTerminalTaskStatus
} from '../../shared/types'
import { buildFeedbackPrompt } from '../../shared/prompt-builder'
import { TASK_TEMPLATE } from './MarkdownEditor'
const RichMarkdownEditor = lazy(() =>
  import('./rich-editor/RichMarkdownEditor').then((m) => ({ default: m.RichMarkdownEditor }))
)
import { AgentPicker } from './AgentPicker'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import { DiffFileList, DiffContent } from './DiffSidebar'
import { CommitDialog } from './CommitDialog'
import { StatusPicker } from './StatusPicker'
import { ProjectPicker } from './ProjectPicker'
import { toast } from './Toast'
import { isWeb } from '../lib/platform'
import {
  X,
  Play,
  Terminal,
  Trash2,
  Archive,
  ArchiveRestore,
  GitBranch,
  Clock,
  Calendar,
  ImagePlus,
  FileCode,
  RefreshCw,
  Loader2,
  GitCommitHorizontal,
  Send,
  MessageSquare,
  ChevronDown,
  ChevronRight,
  FolderGit2,
  Save,
  Workflow,
  Activity
} from 'lucide-react'
import { ConnectorIcon } from './ConnectorIcon'
import { RunEntry } from './workflow-editor/RunEntry'
import { LogReplayModal } from './LogReplayModal'
import { SessionActivityLog } from './SessionActivityLog'
import { ConfirmPopover } from './ConfirmPopover'
import { Tooltip } from './Tooltip'

interface DiffComment {
  filePath: string
  lineIndex: number
  lineContent: string
  comment: string
}

function formatReviewFeedback(comments: DiffComment[]): string {
  const grouped = new Map<string, DiffComment[]>()
  for (const c of comments) {
    if (!grouped.has(c.filePath)) grouped.set(c.filePath, [])
    grouped.get(c.filePath)!.push(c)
  }

  let feedback = 'Please address the following review comments:\n\n'
  for (const [file, fileComments] of grouped) {
    feedback += `**${file}:**\n`
    for (const c of fileComments) {
      const codeLine = c.lineContent.slice(1).trim()
      feedback += `- Line \`${codeLine}\`: ${c.comment}\n`
    }
    feedback += '\n'
  }
  return feedback
}

const EMPTY_TASKS: import('../../shared/types').TaskConfig[] = []
const EMPTY_WORKFLOWS: import('../../shared/types').WorkflowDefinition[] = []

export function TaskDetailPanel() {
  const selectedTaskId = useAppStore((s) => s.selectedTaskId)
  const isCreateMode = selectedTaskId === 'new'
  const task = useAppStore((s) =>
    selectedTaskId && selectedTaskId !== 'new'
      ? (s.config?.tasks || []).find((t) => t.id === selectedTaskId)
      : undefined
  )
  const config = useAppStore((s) => s.config)
  const activeProject = useAppStore((s) => s.activeProject)
  const setSelectedTaskId = useAppStore((s) => s.setSelectedTaskId)
  const removeTask = useAppStore((s) => s.removeTask)
  const archiveTask = useAppStore((s) => s.archiveTask)
  const unarchiveTask = useAppStore((s) => s.unarchiveTask)
  const startTask = useAppStore((s) => s.startTask)
  const addTask = useAppStore((s) => s.addTask)
  const updateTask = useAppStore((s) => s.updateTask)
  const addTerminal = useAppStore((s) => s.addTerminal)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const terminals = useAppStore((s) => s.terminals)
  const allTasks = useAppStore((s) => s.config?.tasks ?? EMPTY_TASKS)
  const workflows = useAppStore((s) => s.config?.workflows ?? EMPTY_WORKFLOWS)

  const [panelWidth, setPanelWidth] = useState(420)
  const [diffResult, setDiffResult] = useState<GitDiffResult | null>(null)
  const [diffLoading, setDiffLoading] = useState(false)
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [showCommitDialog, setShowCommitDialog] = useState(false)
  const [comments, setComments] = useState<DiffComment[]>([])
  const [commentingLine, setCommentingLine] = useState<{
    filePath: string
    lineIndex: number
    lineContent: string
  } | null>(null)
  const [showDiffSection, setShowDiffSection] = useState(true)
  const [showWorkflowRuns, setShowWorkflowRuns] = useState(true)
  const [showSessionActivity, setShowSessionActivity] = useState(true)
  const [sessionLogs, setSessionLogs] = useState<SessionLog[]>([])
  const [fullOutputLogs, setFullOutputLogs] = useState<string | null>(null)

  // Form state (always active for existing tasks + create mode)
  const [formTitle, setFormTitle] = useState('')
  const [formProjectName, setFormProjectName] = useState('')
  const [formDescription, setFormDescription] = useState('')
  const [formBranch, setFormBranch] = useState('')
  const [formUseWorktree, setFormUseWorktree] = useState(false)
  const [formAssignedAgent, setFormAssignedAgent] = useState<AiAgentType | null>(null)
  const [formImages, setFormImages] = useState<string[]>([])
  const [formImagePaths, setFormImagePaths] = useState<Map<string, string>>(new Map())
  const { status: agentInstallStatus } = useAgentInstallStatus()
  const newTaskIdRef = useRef<string>(crypto.randomUUID())
  const initializedRef = useRef(false)
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const taskId = isCreateMode ? newTaskIdRef.current : task?.id

  const project = config?.projects.find((p) => p.name === formProjectName)
  const cwd = task?.worktreePath || project?.path || ''
  const showDiff =
    !isCreateMode && task && (task.status === 'in_review' || task.status === 'in_progress')
  const sessionIsLive = !!(task?.assignedSessionId && terminals.has(task.assignedSessionId))
  const canResume =
    !sessionIsLive &&
    !!task?.agentSessionId &&
    !!task?.assignedAgent &&
    supportsExactSessionResume(task.assignedAgent)

  const [relatedRuns, setRelatedRuns] = useState<(WorkflowExecution & { workflowName?: string })[]>(
    []
  )
  const workflowExecutions = useAppStore((s) => s.workflowExecutions)

  // Reset per-task derived state when the selection changes. React 19's
  // "adjust state during render" pattern avoids set-state-in-effect chains
  // for what is fundamentally a key-on-prop reset.
  const [trackedSelectionId, setTrackedSelectionId] = useState<string | null>(selectedTaskId)
  if (selectedTaskId !== trackedSelectionId) {
    setTrackedSelectionId(selectedTaskId)
    // eslint-disable-next-line react-hooks/refs -- gate the auto-save effect before its next run so it doesn't fire with the new task's data attributed to the old timer
    initializedRef.current = false
    setRelatedRuns([])
    setSessionLogs([])
    setComments([])
    setCommentingLine(null)
    setDiffResult(null)
    setSelectedFile(null)
    if (isCreateMode) {
      // eslint-disable-next-line react-hooks/refs -- regenerate before downstream reads taskId in this same render
      newTaskIdRef.current = crypto.randomUUID()
      setFormTitle('')
      setFormProjectName(activeProject || config?.projects[0]?.name || '')
      setFormDescription(TASK_TEMPLATE)
      setFormBranch('')
      setFormUseWorktree(false)
      setFormAssignedAgent(null)
      setFormImages([])
      setFormImagePaths(new Map())
    } else if (task) {
      setFormTitle(task.title)
      setFormProjectName(task.projectName)
      setFormDescription(task.description)
      setFormBranch(task.branch || '')
      setFormUseWorktree(task.useWorktree || false)
      setFormAssignedAgent(task.assignedAgent || null)
      setFormImages(task.images || [])
      setFormImagePaths(new Map())
    }
  }

  useEffect(() => {
    if (!task) return
    window.api.listWorkflowRunsByTask(task.id, 20).then(setRelatedRuns)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.id])

  useEffect(() => {
    if (!task) return
    const taskId = task.id
    let relevant = false
    for (const [, exec] of workflowExecutions) {
      if (exec.triggerTaskId === taskId || exec.nodeStates.some((ns) => ns.taskId === taskId)) {
        relevant = true
        break
      }
    }
    if (relevant) {
      window.api.listWorkflowRunsByTask(taskId, 20).then(setRelatedRuns)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.id, workflowExecutions])

  useEffect(() => {
    if (!task || isCreateMode) return
    const taskId = task.id
    const fetchLogs = () =>
      window.api.listSessionLogs(taskId).then((next) => {
        setSessionLogs((prev) => {
          if (prev.length !== next.length) return next
          for (let i = 0; i < next.length; i++) {
            if (
              prev[i].sessionId !== next[i].sessionId ||
              prev[i].status !== next[i].status ||
              prev[i].completedAt !== next[i].completedAt ||
              prev[i].exitCode !== next[i].exitCode ||
              (prev[i].logs?.length ?? 0) !== (next[i].logs?.length ?? 0)
            )
              return next
          }
          return prev
        })
      })
    fetchLogs()
    if (task.status === 'in_progress') {
      const interval = setInterval(fetchLogs, 5_000)
      return () => clearInterval(interval)
    }
    return undefined
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.id, task?.status, isCreateMode])

  useEffect(() => {
    if (!task || !task.images?.length) return
    const taskId = task.id
    const images = task.images
    let cancelled = false
    Promise.all(
      images.map(async (f) => {
        const p = await window.api.getTaskImagePath(taskId, f)
        return [f, p] as [string, string]
      })
    ).then((pairs) => {
      if (!cancelled) setFormImagePaths(new Map(pairs))
    })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.id, task?.images])

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      initializedRef.current = true
    })
    return () => cancelAnimationFrame(frame)
  }, [trackedSelectionId])

  // Auto-save for existing tasks (debounced)
  useEffect(() => {
    if (isCreateMode || !task || !initializedRef.current) return

    if (saveTimerRef.current) clearTimeout(saveTimerRef.current)
    saveTimerRef.current = setTimeout(() => {
      updateTask(task.id, {
        title: formTitle.trim(),
        projectName: formProjectName,
        description: formDescription.trim(),
        branch: formBranch.trim() || undefined,
        useWorktree: formUseWorktree || undefined,
        assignedAgent: formAssignedAgent || undefined,
        images: formImages.length > 0 ? formImages : undefined
      })
    }, 500)

    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    formTitle,
    formProjectName,
    formDescription,
    formBranch,
    formUseWorktree,
    formAssignedAgent,
    formImages
  ])

  // Flush pending save on unmount or task switch
  const formRef = useRef({
    formTitle,
    formProjectName,
    formDescription,
    formBranch,
    formUseWorktree,
    formAssignedAgent,
    formImages
  })
  useLayoutEffect(() => {
    formRef.current = {
      formTitle,
      formProjectName,
      formDescription,
      formBranch,
      formUseWorktree,
      formAssignedAgent,
      formImages
    }
  })

  useEffect(() => {
    const taskIdForCleanup = selectedTaskId
    return () => {
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current)
        saveTimerRef.current = null
        // Flush the pending save immediately with latest form state
        if (taskIdForCleanup && taskIdForCleanup !== 'new') {
          const f = formRef.current
          useAppStore.getState().updateTask(taskIdForCleanup, {
            title: f.formTitle.trim(),
            projectName: f.formProjectName,
            description: f.formDescription.trim(),
            branch: f.formBranch.trim() || undefined,
            useWorktree: f.formUseWorktree || undefined,
            assignedAgent: f.formAssignedAgent || undefined,
            images: f.formImages.length > 0 ? f.formImages : undefined
          })
        }
      }
    }
  }, [selectedTaskId])

  // Fetch diff for review tasks
  const fetchDiff = useCallback(async () => {
    if (!cwd || !showDiff) return
    setDiffLoading(true)
    try {
      const result = await window.api.getGitDiffFull(cwd)
      setDiffResult(result)
      setSelectedFile(null)
      setComments([])
      setCommentingLine(null)
    } finally {
      setDiffLoading(false)
    }
  }, [cwd, showDiff])

  useEffect(() => {
    if (selectedTaskId && selectedTaskId !== 'new' && cwd && showDiff) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- fetchDiff is also bound to manual-refresh buttons; refresh on selection/cwd/status change is intentional
      fetchDiff()
    } else {
      setDiffResult(null)
      setSelectedFile(null)
      setComments([])
      setCommentingLine(null)
    }
  }, [selectedTaskId, cwd, showDiff, fetchDiff])

  if (!task && !isCreateMode) return null

  const handleResizeStart = (e: React.PointerEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startWidth = panelWidth
    const onMove = (ev: PointerEvent) => {
      const delta = startX - ev.clientX
      setPanelWidth(Math.max(320, Math.min(600, startWidth + delta)))
    }
    const onUp = () => {
      document.removeEventListener('pointermove', onMove)
      document.removeEventListener('pointerup', onUp)
    }
    document.addEventListener('pointermove', onMove)
    document.addEventListener('pointerup', onUp)
  }

  const handleFocusSession = () => {
    if (task?.assignedSessionId) setFocusedTerminal(task.assignedSessionId)
  }

  const handleResumeSession = async () => {
    if (!task?.agentSessionId || !task?.assignedAgent || !project) return
    if (!supportsExactSessionResume(task.assignedAgent)) return

    const remoteHostId = getProjectRemoteHostId(project)
    const session = await window.api.createTerminal({
      agentType: task.assignedAgent,
      projectName: task.projectName,
      projectPath: project.path,
      branch: task.branch,
      useWorktree: task.useWorktree,
      resumeSessionId: task.agentSessionId,
      remoteHostId
    })
    addTerminal(session)
    if (task.status === 'in_progress') {
      startTask(task.id, session.id, task.assignedAgent as AiAgentType)
    }
    setFocusedTerminal(session.id)
  }

  const handleRunResumeSession = async (
    agentSessionId: string,
    agentType: AiAgentType,
    projectName: string,
    projectPath: string,
    branch?: string,
    useWorktree?: boolean
  ) => {
    if (!supportsExactSessionResume(agentType)) return

    const proj = config?.projects.find((p) => p.name === projectName)
    const remoteHostId = proj ? getProjectRemoteHostId(proj) : undefined
    const session = await window.api.createTerminal({
      agentType,
      projectName,
      projectPath,
      branch,
      useWorktree,
      resumeSessionId: agentSessionId,
      remoteHostId
    })
    addTerminal(session)
    setFocusedTerminal(session.id)
  }

  const handleClickLine = (filePath: string, lineIndex: number, lineContent: string) => {
    if (commentingLine?.filePath === filePath && commentingLine?.lineIndex === lineIndex) {
      setCommentingLine(null)
    } else {
      setCommentingLine({ filePath, lineIndex, lineContent })
    }
  }

  const handleAddComment = (text: string) => {
    if (!commentingLine) return
    setComments((prev) => [
      ...prev,
      {
        filePath: commentingLine.filePath,
        lineIndex: commentingLine.lineIndex,
        lineContent: commentingLine.lineContent,
        comment: text
      }
    ])
    setCommentingLine(null)
  }

  const handleSendFeedback = async () => {
    if (comments.length === 0 || !task) return
    const feedback = formatReviewFeedback(comments)

    if (
      task.agentSessionId &&
      task.assignedAgent &&
      project &&
      supportsExactSessionResume(task.assignedAgent)
    ) {
      const remoteHostId = getProjectRemoteHostId(project)
      const session = await window.api.createTerminal({
        agentType: task.assignedAgent,
        projectName: task.projectName,
        projectPath: project.path,
        branch: task.branch,
        useWorktree: task.useWorktree,
        resumeSessionId: task.agentSessionId,
        initialPrompt: buildFeedbackPrompt(feedback, task, project),
        taskId: task.id,
        remoteHostId
      })
      addTerminal(session)
      startTask(task.id, session.id, task.assignedAgent, session.worktreePath)
      setFocusedTerminal(session.id)
      toast.success('Feedback sent to agent')
    } else {
      await navigator.clipboard.writeText(feedback)
      toast.info('Review feedback copied to clipboard')
    }
    setComments([])
    setCommentingLine(null)
  }

  // Image handlers
  const handleAddImages = async () => {
    if (!taskId) return
    const filePaths = await window.api.openImageDialog()
    if (!filePaths) return

    const newImages = [...formImages]
    const newPaths = new Map(formImagePaths)

    for (const sourcePath of filePaths) {
      const filename = await window.api.saveTaskImage(taskId, sourcePath)
      newImages.push(filename)
      const absPath = await window.api.getTaskImagePath(taskId, filename)
      newPaths.set(filename, absPath)
    }

    setFormImages(newImages)
    setFormImagePaths(newPaths)
  }

  const handleRemoveImage = async (filename: string) => {
    if (!taskId) return
    await window.api.deleteTaskImage(taskId, filename)
    setFormImages((prev) => prev.filter((f) => f !== filename))
    setFormImagePaths((prev) => {
      const next = new Map(prev)
      next.delete(filename)
      return next
    })
  }

  const handleDrop = async (e: React.DragEvent) => {
    if (!taskId) return
    e.preventDefault()
    const files = Array.from(e.dataTransfer.files).filter((f) =>
      /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(f.name)
    )
    if (!files.length) return

    const newImages = [...formImages]
    const newPaths = new Map(formImagePaths)

    for (const file of files) {
      const filename = await window.api.saveTaskImage(
        taskId,
        (file as File & { path: string }).path
      )
      newImages.push(filename)
      const absPath = await window.api.getTaskImagePath(taskId, filename)
      newPaths.set(filename, absPath)
    }

    setFormImages(newImages)
    setFormImagePaths(newPaths)
  }

  const handleCreate = () => {
    if (!formTitle.trim() || !formProjectName || !formDescription.trim()) return

    const now = new Date().toISOString()
    const existingTasks =
      config?.tasks?.filter((t) => t.projectName === formProjectName && t.status === 'todo') || []
    const newId = newTaskIdRef.current
    addTask({
      id: newId,
      projectName: formProjectName,
      title: formTitle.trim(),
      description: formDescription.trim(),
      status: 'todo',
      order: existingTasks.length,
      branch: formBranch.trim() || undefined,
      useWorktree: formUseWorktree || undefined,
      assignedAgent: formAssignedAgent || undefined,
      images: formImages.length > 0 ? formImages : undefined,
      createdAt: now,
      updatedAt: now
    })
    toast.success('Task created')
    setSelectedTaskId(newId)
  }

  const canSubmit = formTitle.trim() && formProjectName && formDescription.trim()

  const stat = diffResult
    ? {
        filesChanged: diffResult.files.length,
        insertions: diffResult.files.reduce((s, f) => s + f.insertions, 0),
        deletions: diffResult.files.reduce((s, f) => s + f.deletions, 0)
      }
    : null

  const hasChanges = stat && (stat.insertions > 0 || stat.deletions > 0)

  const formatDate = (dateStr: string) => {
    const d = new Date(dateStr)
    return d.toLocaleDateString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit'
    })
  }

  const hasSessionActions = !isCreateMode && task && (sessionIsLive || canResume)

  return (
    <div
      className="shrink-0 flex flex-col border-l border-white/[0.08] overflow-hidden"
      style={{ width: panelWidth, background: '#141416' }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={handleDrop}
    >
      {/* Resize handle */}
      <div
        className="absolute left-0 top-0 bottom-0 w-1 cursor-col-resize hover:bg-blue-500/30 transition-colors z-10"
        style={{ position: 'relative', width: 2, minWidth: 2 }}
        onPointerDown={handleResizeStart}
      />

      {/* Header */}
      <div className="px-4 py-3 border-b border-white/[0.06] shrink-0">
        <div className="flex items-center gap-2">
          {isCreateMode ? (
            <h3 className="text-[14px] font-medium text-gray-100 flex-1">New Task</h3>
          ) : (
            <div className="flex-1" />
          )}
          {!isCreateMode && task && task.archivedAt && (
            <Tooltip label="Unarchive task" position="bottom">
              <button
                onClick={() => {
                  unarchiveTask(task.id)
                  toast.success('Task unarchived')
                }}
                className="p-1 text-gray-600 hover:text-gray-200 rounded transition-colors"
              >
                <ArchiveRestore size={13} strokeWidth={1.5} />
              </button>
            </Tooltip>
          )}
          {!isCreateMode && task && !task.archivedAt && isTerminalTaskStatus(task.status) && (
            <Tooltip label="Archive task" position="bottom">
              <button
                onClick={() => {
                  archiveTask(task.id)
                  toast.success('Task archived')
                }}
                className="p-1 text-gray-600 hover:text-gray-200 rounded transition-colors"
              >
                <Archive size={13} strokeWidth={1.5} />
              </button>
            </Tooltip>
          )}
          {!isCreateMode && task && (
            <ConfirmPopover
              message="Delete this task permanently?"
              confirmLabel="Delete"
              onConfirm={() => {
                removeTask(task.id)
                setSelectedTaskId(null)
                toast.success('Task deleted')
              }}
            >
              <Tooltip label="Delete task" position="bottom">
                <button className="p-1 text-gray-600 hover:text-red-400 rounded transition-colors">
                  <Trash2 size={13} strokeWidth={1.5} />
                </button>
              </Tooltip>
            </ConfirmPopover>
          )}
          <Tooltip label="Close" position="bottom">
            <button
              onClick={() => setSelectedTaskId(null)}
              className="p-1 text-gray-500 hover:text-white rounded transition-colors"
            >
              <X size={14} strokeWidth={1.5} />
            </button>
          </Tooltip>
        </div>
      </div>

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto">
        {/* Properties section */}
        <div className="px-4 py-3 border-b border-white/[0.06] space-y-2.5">
          {/* Status */}
          <div className="flex items-center gap-2 text-[12px]">
            <span className="text-gray-600 w-20 shrink-0">Status</span>
            <StatusPicker
              taskId={task?.id}
              currentStatus={task?.status ?? 'todo'}
              disabled={isCreateMode}
            />
          </div>

          {/* Project */}
          <div className="flex items-center gap-2 text-[12px]">
            <span className="text-gray-600 w-20 shrink-0">Project</span>
            <ProjectPicker
              currentProject={formProjectName}
              projects={config?.projects || []}
              onChange={setFormProjectName}
            />
          </div>

          {/* Branch */}
          <div className="flex items-center gap-2 text-[12px]">
            <span className="text-gray-600 w-20 shrink-0">Branch</span>
            <div className="flex items-center gap-1.5 flex-1 min-w-0">
              <GitBranch size={11} strokeWidth={2} className="text-gray-500 shrink-0" />
              <input
                type="text"
                placeholder="feature/my-task"
                value={formBranch}
                onChange={(e) => setFormBranch(e.target.value)}
                className="flex-1 min-w-0 bg-transparent text-[12px] text-gray-300 placeholder-gray-600
                           focus:outline-none border-none px-0 py-0.5"
              />
            </div>
          </div>

          {/* Worktree */}
          <div className="flex items-center gap-2 text-[12px]">
            <span className="text-gray-600 w-20 shrink-0">Worktree</span>
            <button
              onClick={() => setFormUseWorktree(!formUseWorktree)}
              className={`flex items-center gap-1.5 hover:bg-white/[0.04] rounded px-1.5 py-0.5 -mx-1.5 transition-colors ${
                formUseWorktree ? 'text-amber-400' : 'text-gray-500'
              }`}
            >
              <FolderGit2 size={13} strokeWidth={1.5} />
              <span className="text-[12px]">{formUseWorktree ? 'Enabled' : 'Disabled'}</span>
            </button>
          </div>

          {/* Agent */}
          <div className="flex items-center gap-2 text-[12px]">
            <span className="text-gray-600 w-20 shrink-0">Agent</span>
            <AgentPicker
              currentAgent={formAssignedAgent}
              onChange={(a) => setFormAssignedAgent(a === 'fromTask' ? null : a)}
              installStatus={agentInstallStatus}
              allowNone
            />
          </div>

          {/* Created */}
          {!isCreateMode && task && (
            <div className="flex items-center gap-2 text-[12px]">
              <span className="text-gray-600 w-20 shrink-0">Created</span>
              <span className="flex items-center gap-1 text-gray-400">
                <Calendar size={11} strokeWidth={2} />
                {formatDate(task.createdAt)}
              </span>
            </div>
          )}

          {/* Completed */}
          {!isCreateMode && task?.completedAt && (
            <div className="flex items-center gap-2 text-[12px]">
              <span className="text-gray-600 w-20 shrink-0">Completed</span>
              <span className="flex items-center gap-1 text-gray-400">
                <Clock size={11} strokeWidth={2} />
                {formatDate(task.completedAt)}
              </span>
            </div>
          )}
        </div>

        {/* Title */}
        <div className="px-4 pt-4 pb-2">
          <input
            type="text"
            placeholder="Task title"
            value={formTitle}
            onChange={(e) => setFormTitle(e.target.value)}
            autoFocus={isCreateMode}
            className="w-full text-[16px] font-semibold text-gray-100 bg-transparent
                       border-none outline-none placeholder-gray-600
                       focus:bg-white/[0.02] rounded px-1 -mx-1 py-0.5 transition-colors"
          />
        </div>

        {/* Source reference — real href so middle-click, copy-link, and
             screen readers announce the destination. Electron opens it in
             the system browser via openExternal. */}
        {task?.sourceConnectorId && task.sourceExternalUrl && (
          <div className="px-4 pb-2">
            <a
              href={task.sourceExternalUrl}
              onClick={(e) => {
                e.preventDefault()
                window.api.openExternal(task.sourceExternalUrl!)
              }}
              className="inline-flex items-center gap-1.5 px-2 py-1 bg-white/[0.04] border border-white/[0.08] rounded-md text-xs text-gray-400 hover:text-white hover:bg-white/[0.08] transition-colors"
            >
              <ConnectorIcon
                connectorId={task.sourceConnectorId}
                size={13}
                className="text-gray-400"
              />
              {task.sourceExternalId ? `#${task.sourceExternalId}` : task.sourceConnectorId}
              <span className="text-gray-600">↗</span>
            </a>
          </div>
        )}

        {/* Description */}
        <div className="px-4 pb-3">
          <Suspense
            fallback={<div className="h-[120px] bg-white/[0.03] rounded-lg animate-pulse" />}
          >
            <RichMarkdownEditor
              value={formDescription}
              onChange={setFormDescription}
              placeholder="Describe the task in detail, or type / for commands..."
            />
          </Suspense>
        </div>

        {/* Images */}
        <div className="px-4 pb-3">
          <div className="flex flex-wrap gap-2">
            {formImages.map((filename) => {
              const absPath = formImagePaths.get(filename)
              return (
                <div
                  key={filename}
                  className="relative group/img w-16 h-16 rounded-lg border border-white/[0.08] overflow-hidden bg-white/[0.03]"
                >
                  {absPath && (
                    <img
                      src={isWeb ? absPath : `file://${absPath}`}
                      alt=""
                      className="w-full h-full object-cover"
                    />
                  )}
                  <button
                    onClick={() => handleRemoveImage(filename)}
                    className="absolute top-0.5 right-0.5 w-4 h-4 rounded-full bg-black/70 flex items-center justify-center
                                 opacity-0 group-hover/img:opacity-100 transition-opacity text-white hover:text-red-400"
                  >
                    <X size={10} strokeWidth={3} />
                  </button>
                </div>
              )
            })}
            <button
              onClick={handleAddImages}
              className="w-16 h-16 rounded-lg border border-dashed border-white/[0.1] flex items-center justify-center
                           text-gray-600 hover:text-gray-400 hover:border-white/[0.2] transition-colors"
              title="Add images"
            >
              <ImagePlus size={18} strokeWidth={1.5} />
            </button>
          </div>
        </div>

        {/* Session action buttons (compact) */}
        {hasSessionActions && (
          <div className="px-4 py-2 border-t border-white/[0.06] flex flex-wrap gap-2">
            {sessionIsLive && (
              <button
                onClick={handleFocusSession}
                className="flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] font-medium
                           bg-violet-500/10 hover:bg-violet-500/20 border border-violet-500/20
                           rounded-md transition-colors text-violet-400 hover:text-violet-300"
              >
                <Terminal size={12} strokeWidth={2} />
                Focus Session
              </button>
            )}
            {canResume && (
              <button
                onClick={handleResumeSession}
                className="flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] font-medium
                           bg-amber-500/10 hover:bg-amber-500/20 border border-amber-500/20
                           rounded-md transition-colors text-amber-400 hover:text-amber-300"
              >
                <Play size={12} strokeWidth={2} />
                Resume Session
              </button>
            )}
          </div>
        )}

        {/* Session Activity section */}
        {!isCreateMode && sessionLogs.length > 0 && (
          <div className="border-t border-white/[0.06]">
            <button
              onClick={() => setShowSessionActivity(!showSessionActivity)}
              className="w-full px-4 py-2.5 flex items-center gap-2 text-[11px] font-medium text-gray-500
                         uppercase tracking-wider hover:text-gray-300 transition-colors"
            >
              {showSessionActivity ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              <Activity size={12} strokeWidth={2} />
              Session Activity ({sessionLogs.length})
            </button>

            {showSessionActivity && (
              <div className="px-3 pb-3">
                <SessionActivityLog
                  logs={sessionLogs}
                  onViewFullOutput={setFullOutputLogs}
                  onResumeSession={handleRunResumeSession}
                  agentSessionId={task?.agentSessionId}
                  projectPath={project?.path}
                />
              </div>
            )}
          </div>
        )}

        {/* Workflow Runs section */}
        {!isCreateMode && relatedRuns.length > 0 && (
          <div className="border-t border-white/[0.06]">
            <button
              onClick={() => setShowWorkflowRuns(!showWorkflowRuns)}
              className="w-full px-4 py-2.5 flex items-center gap-2 text-[11px] font-medium text-gray-500
                         uppercase tracking-wider hover:text-gray-300 transition-colors"
            >
              {showWorkflowRuns ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              <Workflow size={12} strokeWidth={2} />
              Workflow Runs ({relatedRuns.length})
            </button>

            {showWorkflowRuns && (
              <div className="px-3 pb-3 space-y-2">
                {relatedRuns.map((run, i) => {
                  const wf = workflows.find((w) => w.id === run.workflowId)
                  return (
                    <RunEntry
                      key={`${run.workflowId}-${run.startedAt}-${i}`}
                      execution={run}
                      nodes={wf?.nodes || []}
                      workflowName={run.workflowName || wf?.name}
                      tasks={allTasks}
                      onViewFullOutput={setFullOutputLogs}
                      onResumeSession={handleRunResumeSession}
                    />
                  )
                })}
              </div>
            )}
          </div>
        )}

        {/* Diff review section */}
        {showDiff && (
          <div className="border-t border-white/[0.06]">
            <button
              onClick={() => setShowDiffSection(!showDiffSection)}
              className="w-full px-4 py-2.5 flex items-center gap-2 text-[11px] font-medium text-gray-500
                         uppercase tracking-wider hover:text-gray-300 transition-colors"
            >
              {showDiffSection ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              <FileCode size={12} strokeWidth={2} />
              Changes
              {stat && (
                <span className="flex items-center gap-1.5 font-mono normal-case">
                  <span className="text-green-400">+{stat.insertions}</span>
                  <span className="text-red-400">-{stat.deletions}</span>
                </span>
              )}
              <div className="flex-1" />
              {hasChanges && (
                <span
                  onClick={(e) => {
                    e.stopPropagation()
                    setShowCommitDialog(true)
                  }}
                  className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-medium normal-case
                             bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06]
                             rounded transition-colors text-gray-400 hover:text-gray-200"
                >
                  <GitCommitHorizontal size={11} strokeWidth={1.5} />
                  Commit
                </span>
              )}
              <span
                onClick={(e) => {
                  e.stopPropagation()
                  fetchDiff()
                }}
                className="p-0.5 text-gray-500 hover:text-white rounded transition-colors"
              >
                <RefreshCw
                  size={12}
                  className={diffLoading ? 'animate-spin' : ''}
                  strokeWidth={1.5}
                />
              </span>
            </button>

            {showDiffSection && (
              <>
                {/* Review feedback bar */}
                {comments.length > 0 && (
                  <div className="px-3 py-2 border-t border-purple-500/15 bg-purple-500/[0.05] flex items-center gap-2">
                    <MessageSquare size={13} className="text-purple-400 shrink-0" />
                    <span className="text-[12px] text-purple-300 flex-1">
                      {comments.length} comment{comments.length !== 1 ? 's' : ''}
                    </span>
                    <button
                      onClick={() => setComments([])}
                      className="text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
                    >
                      Clear
                    </button>
                    <button
                      onClick={handleSendFeedback}
                      className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-medium
                                 bg-purple-500/20 hover:bg-purple-500/30 border border-purple-500/20
                                 rounded-md transition-colors text-purple-300 hover:text-purple-200"
                    >
                      <Send size={11} strokeWidth={2} />
                      {task?.agentSessionId ? 'Send to Agent' : 'Copy Feedback'}
                    </button>
                  </div>
                )}

                {diffLoading && !diffResult ? (
                  <div className="flex items-center justify-center py-8">
                    <Loader2 size={18} className="text-gray-500 animate-spin" />
                  </div>
                ) : diffResult && diffResult.files.length > 0 ? (
                  <>
                    <DiffFileList
                      files={diffResult.files}
                      selectedFile={selectedFile}
                      onSelectFile={setSelectedFile}
                    />
                    <DiffContent
                      files={diffResult.files}
                      selectedFile={selectedFile}
                      comments={comments}
                      commentingLine={commentingLine}
                      onClickLine={handleClickLine}
                      onAddComment={handleAddComment}
                      onCancelComment={() => setCommentingLine(null)}
                      onRemoveComment={(idx) =>
                        setComments((prev) => prev.filter((_, i) => i !== idx))
                      }
                    />
                  </>
                ) : (
                  <div className="text-center py-6 text-xs text-gray-600">
                    {cwd ? 'No uncommitted changes' : 'No project path'}
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>

      {/* Footer: only for create mode */}
      {isCreateMode && (
        <div className="px-4 py-3 border-t border-white/[0.06] flex justify-end gap-2 shrink-0">
          <button
            onClick={() => setSelectedTaskId(null)}
            className="px-3 py-1.5 text-sm text-gray-400 hover:text-gray-200
                       bg-white/[0.04] hover:bg-white/[0.08] rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            disabled={!canSubmit}
            className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium text-white
                       bg-white/[0.1] hover:bg-white/[0.15]
                       disabled:opacity-30 disabled:cursor-not-allowed
                       rounded-lg transition-colors"
          >
            <Save size={13} strokeWidth={2} />
            Create Task
          </button>
        </div>
      )}

      {showCommitDialog && (
        <CommitDialog
          cwd={cwd}
          branch={task?.branch}
          stat={stat ?? undefined}
          onClose={() => setShowCommitDialog(false)}
          onCommitted={() => {
            fetchDiff()
            setShowCommitDialog(false)
          }}
        />
      )}

      {fullOutputLogs !== null && (
        <LogReplayModal logs={fullOutputLogs} onClose={() => setFullOutputLogs(null)} />
      )}
    </div>
  )
}
