import { useState, useEffect, useRef, Suspense, lazy } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { useWorkspaceProjects } from '../hooks/useWorkspaceProjects'
import { FolderGit2, X, Maximize2, Paperclip } from 'lucide-react'
const RichMarkdownEditor = lazy(() =>
  import('./rich-editor/RichMarkdownEditor').then((m) => ({ default: m.RichMarkdownEditor }))
)
import { TASK_TEMPLATE } from './MarkdownEditor'
import { toast } from './Toast'
import { isWeb } from '../lib/platform'
import { StatusPicker } from './StatusPicker'
import { ProjectPicker } from './ProjectPicker'
import { AgentPicker } from './AgentPicker'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import type { AiAgentType, TaskStatus } from '../../shared/types'

export function AddTaskDialog() {
  const isOpen = useAppStore((s) => s.isTaskDialogOpen)
  const setOpen = useAppStore((s) => s.setTaskDialogOpen)
  const editingTask = useAppStore((s) => s.editingTask)
  const setEditingTask = useAppStore((s) => s.setEditingTask)
  const config = useAppStore((s) => s.config)
  const addTask = useAppStore((s) => s.addTask)
  const updateTask = useAppStore((s) => s.updateTask)
  const activeProject = useAppStore((s) => s.activeProject)
  const setSelectedTaskId = useAppStore((s) => s.setSelectedTaskId)
  const storeDefaultStatus = useAppStore((s) => s.taskDialogDefaultStatus)

  const [title, setTitle] = useState('')
  const [status, setStatus] = useState<TaskStatus>(storeDefaultStatus)
  const [projectName, setProjectName] = useState('')
  const [description, setDescription] = useState('')
  const [branch, setBranch] = useState('')
  const [useWorktree, setUseWorktree] = useState(false)
  const [assignedAgent, setAssignedAgent] = useState<AiAgentType | null>(null)
  const [images, setImages] = useState<string[]>([])
  const [imagePaths, setImagePaths] = useState<Map<string, string>>(new Map())
  const taskIdRef = useRef<string>(crypto.randomUUID())
  const { status: agentInstallStatus } = useAgentInstallStatus()

  const workspaceProjects = useWorkspaceProjects()

  const isEditMode = !!editingTask

  const initForm = (editing: typeof editingTask) => {
    if (editing) {
      setTitle(editing.title)
      setProjectName(editing.projectName)
      setDescription(editing.description)
      setBranch(editing.branch || '')
      setUseWorktree(editing.useWorktree || false)
      setAssignedAgent(editing.assignedAgent || null)
      setImages(editing.images || [])
      setStatus(editing.status)
      taskIdRef.current = editing.id
      if (editing.images?.length) {
        Promise.all(
          editing.images.map(async (f) => {
            const p = await window.api.getTaskImagePath(editing.id, f)
            return [f, p] as [string, string]
          })
        ).then((pairs) => setImagePaths(new Map(pairs)))
      }
    } else {
      setProjectName(activeProject || workspaceProjects[0]?.name || '')
      setDescription(TASK_TEMPLATE)
      taskIdRef.current = crypto.randomUUID()
    }
  }

  useEffect(() => {
    if (!isOpen) return
    initForm(editingTask)
    setStatus(storeDefaultStatus)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, editingTask])

  const resetForm = () => {
    setTitle('')
    setProjectName(activeProject || workspaceProjects[0]?.name || '')
    setDescription(TASK_TEMPLATE)
    setBranch('')
    setUseWorktree(false)
    setAssignedAgent(null)
    setImages([])
    setImagePaths(new Map())
    setStatus('todo')
    taskIdRef.current = crypto.randomUUID()
  }

  const handleClose = () => {
    setOpen(false)
    setEditingTask(null)
    resetForm()
  }

  const handleExpand = () => {
    setOpen(false)
    setEditingTask(null)
    setSelectedTaskId('new')
    resetForm()
  }

  const handleAddImages = async () => {
    const filePaths = await window.api.openImageDialog()
    if (!filePaths) return

    const taskId = isEditMode ? editingTask.id : taskIdRef.current
    const newImages = [...images]
    const newPaths = new Map(imagePaths)

    for (const sourcePath of filePaths) {
      const filename = await window.api.saveTaskImage(taskId, sourcePath)
      newImages.push(filename)
      const absPath = await window.api.getTaskImagePath(taskId, filename)
      newPaths.set(filename, absPath)
    }

    setImages(newImages)
    setImagePaths(newPaths)
  }

  const handleRemoveImage = async (filename: string) => {
    const taskId = isEditMode ? editingTask.id : taskIdRef.current
    await window.api.deleteTaskImage(taskId, filename)
    setImages((prev) => prev.filter((f) => f !== filename))
    setImagePaths((prev) => {
      const next = new Map(prev)
      next.delete(filename)
      return next
    })
  }

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault()
    const files = Array.from(e.dataTransfer.files).filter((f) =>
      /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(f.name)
    )
    if (!files.length) return

    const taskId = isEditMode ? editingTask.id : taskIdRef.current
    const newImages = [...images]
    const newPaths = new Map(imagePaths)

    for (const file of files) {
      const filename = await window.api.saveTaskImage(
        taskId,
        (file as File & { path: string }).path
      )
      newImages.push(filename)
      const absPath = await window.api.getTaskImagePath(taskId, filename)
      newPaths.set(filename, absPath)
    }

    setImages(newImages)
    setImagePaths(newPaths)
  }

  const handleSubmit = () => {
    if (!title.trim() || !projectName || !description.trim()) return

    const now = new Date().toISOString()
    if (isEditMode) {
      updateTask(editingTask.id, {
        title: title.trim(),
        projectName,
        description: description.trim(),
        status,
        branch: branch.trim() || undefined,
        useWorktree: useWorktree || undefined,
        assignedAgent: assignedAgent || undefined,
        images: images.length > 0 ? images : undefined
      })
      toast.success('Task updated')
      handleClose()
    } else {
      const existingTasks =
        config?.tasks?.filter((t) => t.projectName === projectName && t.status === status) || []
      addTask({
        id: taskIdRef.current,
        projectName,
        title: title.trim(),
        description: description.trim(),
        status: status,
        order: existingTasks.length,
        branch: branch.trim() || undefined,
        useWorktree: useWorktree || undefined,
        assignedAgent: assignedAgent || undefined,
        images: images.length > 0 ? images : undefined,
        createdAt: now,
        updatedAt: now
      })
      toast.success('Task created')
      handleClose()
    }
  }

  const canSubmit = title.trim() && projectName && description.trim()

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          {/* Light backdrop */}
          <motion.div
            className="fixed inset-0 bg-black/30 z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={handleClose}
          />

          {/* Floating inline form */}
          <motion.div
            className="fixed z-50 w-[560px] border border-white/[0.1]
                       rounded-xl shadow-2xl overflow-hidden flex flex-col"
            style={{ background: 'var(--color-surface-overlay)', left: '50%', top: '50%' }}
            initial={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
            exit={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onDragOver={(e) => e.preventDefault()}
            onDrop={handleDrop}
          >
            {/* Header bar */}
            <div className="flex items-center justify-between px-4 py-2.5 border-b border-white/[0.06]">
              <div className="flex items-center gap-2 text-[13px] text-gray-400">
                <span className="text-gray-500">{isEditMode ? 'Edit Task' : 'New task'}</span>
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={handleExpand}
                  className="p-1 text-gray-500 hover:text-white rounded transition-colors"
                  title="Expand to full panel"
                >
                  <Maximize2 size={14} />
                </button>
                <button
                  onClick={handleClose}
                  className="p-1 text-gray-500 hover:text-white rounded transition-colors"
                  title="Close"
                >
                  <X size={14} />
                </button>
              </div>
            </div>

            {/* Form body */}
            <div className="flex flex-col max-h-[60vh] overflow-auto">
              {/* Title input */}
              <input
                type="text"
                placeholder="Task title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                autoFocus
                className="w-full px-4 pt-3 pb-1 bg-transparent text-[15px] font-medium
                           text-gray-200 placeholder-gray-600 focus:outline-none"
              />

              {/* Description */}
              <div className="px-4 pb-3 min-h-[120px]">
                <Suspense
                  fallback={<div className="h-[120px] bg-white/[0.03] rounded-lg animate-pulse" />}
                >
                  <RichMarkdownEditor
                    value={description}
                    onChange={setDescription}
                    placeholder="Add description..."
                  />
                </Suspense>
              </div>

              {/* Image previews */}
              {images.length > 0 && (
                <div className="px-4 pb-3 flex flex-wrap gap-2">
                  {images.map((filename) => {
                    const absPath = imagePaths.get(filename)
                    return (
                      <div
                        key={filename}
                        className="relative group/img w-14 h-14 rounded-lg border border-white/[0.08] overflow-hidden bg-white/[0.03]"
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
                </div>
              )}
            </div>

            {/* Property pills toolbar */}
            <div className="flex items-center gap-2 px-4 py-2 border-t border-white/[0.06]">
              {/* Status picker */}
              <StatusPicker currentStatus={status} onChange={setStatus} />

              {/* Project picker */}
              <ProjectPicker
                currentProject={projectName}
                projects={workspaceProjects}
                onChange={setProjectName}
              />

              {/* Agent picker */}
              <AgentPicker
                currentAgent={assignedAgent}
                onChange={(a) => setAssignedAgent(a === 'fromTask' ? null : a)}
                installStatus={agentInstallStatus}
                allowNone
              />

              {/* Branch pill */}
              {(branch || isEditMode) && (
                <span className="flex items-center gap-1 px-2 py-1 rounded-full bg-white/[0.06] text-xs text-gray-400">
                  <FolderGit2 size={10} />
                  {branch || 'branch'}
                </span>
              )}

              <div className="flex-1" />

              {/* Attach images */}
              <button
                onClick={handleAddImages}
                className="p-1.5 text-gray-500 hover:text-gray-300 rounded transition-colors"
                title="Attach images"
              >
                <Paperclip size={14} />
              </button>
            </div>

            {/* Footer */}
            <div className="flex items-center justify-end px-4 py-3 border-t border-white/[0.06]">
              <button
                onClick={handleSubmit}
                disabled={!canSubmit}
                className="px-3 py-1.5 text-sm font-medium text-white
                           bg-white/[0.1] hover:bg-white/[0.15]
                           disabled:opacity-30 disabled:cursor-not-allowed
                           rounded-lg transition-colors"
              >
                {isEditMode ? 'Save' : 'Create task'}
              </button>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
