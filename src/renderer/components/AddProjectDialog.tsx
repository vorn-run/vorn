import { useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { AiAgentType } from '../../shared/types'
import { PROJECT_ICON_OPTIONS, ICON_COLOR_PALETTE } from '../lib/project-icons'
import { toast } from './Toast'
import {
  Folder,
  FolderGit2,
  Code,
  Globe,
  Database,
  Server,
  Smartphone,
  Package,
  FileCode,
  Terminal,
  Cpu,
  Cloud,
  Shield,
  Zap,
  Gamepad2,
  Music,
  Image,
  BookOpen,
  FlaskConical,
  Rocket,
  FolderOpen,
  Monitor
} from 'lucide-react'

const ICON_MAP: Record<
  string,
  React.FC<{ size?: number; color?: string; strokeWidth?: number }>
> = {
  Folder,
  FolderGit2,
  Code,
  Globe,
  Database,
  Server,
  Smartphone,
  Package,
  FileCode,
  Terminal,
  Cpu,
  Cloud,
  Shield,
  Zap,
  Gamepad2,
  Music,
  Image,
  BookOpen,
  FlaskConical,
  Rocket
}

export function AddProjectDialog() {
  const isOpen = useAppStore((s) => s.isAddProjectDialogOpen)
  const setOpen = useAppStore((s) => s.setAddProjectDialogOpen)
  const addProject = useAppStore((s) => s.addProject)
  const updateProject = useAppStore((s) => s.updateProject)
  const editingProject = useAppStore((s) => s.editingProject)
  const setEditingProject = useAppStore((s) => s.setEditingProject)
  const config = useAppStore((s) => s.config)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)

  const isEditMode = !!editingProject

  const [selectedPath, setSelectedPath] = useState('')
  const [projectName, setProjectName] = useState('')
  const [selectedIcon, setSelectedIcon] = useState('Folder')
  const [selectedColor, setSelectedColor] = useState('#6b7280')
  // 'local' or a remote host UUID
  const [selectedHostId, setSelectedHostId] = useState<string>('local')
  const [prevOpen, setPrevOpen] = useState(false)

  const remoteHosts = config?.remoteHosts ?? []
  const hasRemoteHosts = remoteHosts.length > 0
  const isRemote = selectedHostId !== 'local'

  // Sync form fields when dialog opens for editing
  if (isOpen && !prevOpen && editingProject) {
    setProjectName(editingProject.name)
    setSelectedPath(editingProject.path)
    setSelectedIcon(editingProject.icon || 'Folder')
    setSelectedColor(editingProject.iconColor || '#6b7280')
    const hostIds = editingProject.hostIds?.length ? editingProject.hostIds : ['local']
    const remoteId = hostIds.find((id) => id !== 'local')
    setSelectedHostId(remoteId || 'local')
  }
  if (isOpen !== prevOpen) {
    setPrevOpen(isOpen)
  }

  const handleBrowse = async (): Promise<void> => {
    const path = await window.api.openDirectoryDialog()
    if (path) {
      setSelectedPath(path)
      if (!projectName) {
        const folderName = path.split('/').pop() || ''
        setProjectName(folderName)
      }
    }
  }

  const handleSubmit = (): void => {
    if (!projectName.trim() || !selectedPath.trim()) return

    const project = {
      name: projectName.trim(),
      path: selectedPath.trim(),
      preferredAgents: (editingProject?.preferredAgents || ['claude']) as AiAgentType[],
      icon: selectedIcon,
      iconColor: selectedColor,
      hostIds: [selectedHostId],
      workspaceId: editingProject?.workspaceId ?? activeWorkspace
    }

    if (isEditMode) {
      updateProject(editingProject.name, project)
      toast.success(`Project "${project.name}" updated`)
    } else {
      addProject(project)
      toast.success(`Project "${project.name}" added`)
    }
    handleClose()
  }

  const handleClose = (): void => {
    setOpen(false)
    setEditingProject(null)
    setSelectedPath('')
    setProjectName('')
    setSelectedIcon('Folder')
    setSelectedColor('#6b7280')
    setSelectedHostId('local')
  }

  const SelectedIconComponent = ICON_MAP[selectedIcon] || Folder

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div
            className="fixed inset-0 bg-black/50 z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={handleClose}
          />

          <motion.div
            className="fixed top-1/2 left-1/2 z-50 w-[520px] border border-white/[0.08]
                       rounded-xl shadow-2xl overflow-hidden"
            style={{ background: 'var(--color-surface-overlay)' }}
            initial={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
            exit={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          >
            {/* Header */}
            <div className="px-6 py-4 border-b border-white/[0.06]">
              <h2 className="text-lg font-medium text-white">
                {isEditMode ? 'Edit Project' : 'Add Project'}
              </h2>
              <p className="text-sm text-gray-500 mt-0.5">
                {isEditMode
                  ? 'Update your project settings'
                  : 'Select a folder and customize your project'}
              </p>
            </div>

            <div className="p-6 space-y-5">
              {/* Host selector — only when remote hosts exist */}
              {hasRemoteHosts && (
                <div>
                  <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                    Host
                  </label>
                  <div className="flex flex-wrap gap-2">
                    <button
                      onClick={() => setSelectedHostId('local')}
                      className={`flex items-center gap-2 px-3 py-2 rounded-lg border text-sm transition-all ${
                        !isRemote
                          ? 'border-white/[0.15] bg-white/[0.06] text-white'
                          : 'border-white/[0.04] bg-white/[0.02] text-gray-500 hover:border-white/[0.1]'
                      }`}
                    >
                      <Monitor
                        size={13}
                        strokeWidth={1.5}
                        className={!isRemote ? 'text-white' : 'text-gray-500'}
                      />
                      Local
                    </button>
                    {remoteHosts.map((host) => (
                      <button
                        key={host.id}
                        onClick={() => setSelectedHostId(host.id)}
                        className={`flex items-center gap-2 px-3 py-2 rounded-lg border text-sm transition-all ${
                          selectedHostId === host.id
                            ? 'border-blue-500/20 bg-blue-500/[0.06] text-blue-300'
                            : 'border-white/[0.04] bg-white/[0.02] text-gray-500 hover:border-white/[0.1]'
                        }`}
                      >
                        <Server
                          size={13}
                          className={selectedHostId === host.id ? 'text-blue-400' : 'text-gray-500'}
                          strokeWidth={1.5}
                        />
                        {host.label}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Folder picker / Remote path */}
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                  {isRemote ? 'Remote Path' : 'Project Folder'}
                </label>
                {!isRemote ? (
                  <div className="flex gap-2">
                    <button
                      onClick={handleBrowse}
                      className="flex items-center gap-2 px-4 py-2.5 bg-white/[0.06] hover:bg-white/[0.1]
                                 border border-white/[0.08] rounded-lg text-sm text-gray-200
                                 transition-colors shrink-0"
                    >
                      <FolderOpen size={16} strokeWidth={1.5} />
                      Browse
                    </button>
                    <div
                      className="flex-1 px-4 py-2.5 bg-white/[0.03] border border-white/[0.06]
                                    rounded-lg text-sm text-gray-400 truncate min-w-0"
                    >
                      {selectedPath || 'No folder selected'}
                    </div>
                  </div>
                ) : (
                  <input
                    type="text"
                    placeholder="/home/user/project"
                    value={selectedPath}
                    onChange={(e) => {
                      setSelectedPath(e.target.value)
                      if (!projectName) {
                        const folderName = e.target.value.split('/').pop() || ''
                        setProjectName(folderName)
                      }
                    }}
                    className="w-full px-4 py-2.5 bg-white/[0.03] border border-white/[0.06] rounded-lg text-sm
                               text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                  />
                )}
              </div>

              {/* Project name */}
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                  Project Name
                </label>
                <input
                  type="text"
                  placeholder="my-project"
                  value={projectName}
                  onChange={(e) => setProjectName(e.target.value)}
                  className="w-full px-4 py-2.5 bg-white/[0.03] border border-white/[0.06] rounded-lg text-sm
                             text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                />
              </div>

              {/* Icon selector */}
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                  Icon
                </label>
                <div className="grid grid-cols-10 gap-1.5">
                  {PROJECT_ICON_OPTIONS.map((opt) => {
                    const IconComp = ICON_MAP[opt.name] || Folder
                    return (
                      <button
                        key={opt.name}
                        onClick={() => setSelectedIcon(opt.name)}
                        className={`flex items-center justify-center p-2 rounded-lg border transition-all ${
                          selectedIcon === opt.name
                            ? 'border-white/[0.2] bg-white/[0.08]'
                            : 'border-transparent hover:bg-white/[0.04]'
                        }`}
                        title={opt.label}
                      >
                        <IconComp
                          size={16}
                          color={selectedIcon === opt.name ? selectedColor : '#9ca3af'}
                          strokeWidth={1.5}
                        />
                      </button>
                    )
                  })}
                </div>
              </div>

              {/* Color selector */}
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                  Color
                </label>
                <div className="flex gap-2 items-center">
                  {ICON_COLOR_PALETTE.map((color) => (
                    <button
                      key={color}
                      onClick={() => setSelectedColor(color)}
                      className={`w-7 h-7 rounded-full border-2 transition-all ${
                        selectedColor === color
                          ? 'border-white scale-110'
                          : 'border-transparent hover:border-white/30'
                      }`}
                      style={{ backgroundColor: color }}
                    />
                  ))}
                  <div className="ml-3 flex items-center gap-2">
                    <SelectedIconComponent size={20} color={selectedColor} strokeWidth={1.5} />
                    <span className="text-xs text-gray-500">Preview</span>
                  </div>
                </div>
              </div>
            </div>

            {/* Footer */}
            <div className="px-6 py-4 border-t border-white/[0.06] flex justify-end gap-3">
              <button
                onClick={handleClose}
                className="px-4 py-2 text-sm text-gray-400 hover:text-gray-200
                           bg-white/[0.04] hover:bg-white/[0.08] rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleSubmit}
                disabled={!projectName.trim() || !selectedPath.trim()}
                className="px-4 py-2 text-sm font-medium text-white
                           bg-white/[0.1] hover:bg-white/[0.15]
                           disabled:opacity-30 disabled:cursor-not-allowed
                           rounded-lg transition-colors"
              >
                {isEditMode ? 'Save' : 'Add Project'}
              </button>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
