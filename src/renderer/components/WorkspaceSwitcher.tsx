import { useState, useRef, useEffect } from 'react'
import { useAppStore } from '../stores'
import { WorkspaceConfig, DEFAULT_WORKSPACE } from '../../shared/types'
import { ICON_COLOR_PALETTE } from '../lib/project-icons'
import { toast } from './Toast'
import {
  ChevronDown,
  Plus,
  Pencil,
  Trash2,
  Check,
  X,
  User,
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
  Briefcase,
  Building2,
  Users,
  Star,
  Heart,
  Coffee,
  Layers
} from 'lucide-react'

const ICON_MAP: Record<
  string,
  React.FC<{ size?: number; color?: string; strokeWidth?: number }>
> = {
  User,
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
  Briefcase,
  Building2,
  Users,
  Star,
  Heart,
  Coffee,
  Layers
}

const WORKSPACE_ICON_OPTIONS = [
  { name: 'User', label: 'Personal' },
  { name: 'Briefcase', label: 'Work' },
  { name: 'Building2', label: 'Company' },
  { name: 'Users', label: 'Team' },
  { name: 'Code', label: 'Code' },
  { name: 'Star', label: 'Star' },
  { name: 'Heart', label: 'Favorite' },
  { name: 'Coffee', label: 'Side Project' },
  { name: 'Layers', label: 'Layers' },
  { name: 'Rocket', label: 'Launch' },
  { name: 'FlaskConical', label: 'Lab' },
  { name: 'Globe', label: 'Web' },
  { name: 'Shield', label: 'Security' },
  { name: 'Zap', label: 'Fast' },
  { name: 'Gamepad2', label: 'Game' },
  { name: 'BookOpen', label: 'Docs' }
]

function WorkspaceIcon({
  icon,
  color,
  size = 14
}: {
  icon?: string
  color?: string
  size?: number
}) {
  const IconComp = icon ? ICON_MAP[icon] : User
  if (IconComp) {
    return <IconComp size={size} color={color || '#6b7280'} strokeWidth={1.5} />
  }
  return <User size={size} color={color || '#6b7280'} strokeWidth={1.5} />
}

export function WorkspaceSwitcher() {
  const config = useAppStore((s) => s.config)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const setActiveWorkspace = useAppStore((s) => s.setActiveWorkspace)
  const addWorkspace = useAppStore((s) => s.addWorkspace)
  const removeWorkspace = useAppStore((s) => s.removeWorkspace)
  const updateWorkspace = useAppStore((s) => s.updateWorkspace)

  const [isOpen, setIsOpen] = useState(false)
  const [isCreating, setIsCreating] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [formName, setFormName] = useState('')
  const [formIcon, setFormIcon] = useState('Briefcase')
  const [formColor, setFormColor] = useState('#3b82f6')
  const [showIconPicker, setShowIconPicker] = useState(false)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  const inputRef = useRef<HTMLInputElement>(null)

  const workspaces = config?.workspaces ?? [DEFAULT_WORKSPACE]
  const currentWorkspace = workspaces.find((ws) => ws.id === activeWorkspace) ?? DEFAULT_WORKSPACE
  const projects = config?.projects ?? []

  const resetForm = () => {
    setIsCreating(false)
    setEditingId(null)
    setFormName('')
    setFormIcon('Briefcase')
    setFormColor('#3b82f6')
    setShowIconPicker(false)
    setConfirmDeleteId(null)
  }

  useEffect(() => {
    if ((isCreating || editingId) && inputRef.current) {
      inputRef.current.focus()
    }
  }, [isCreating, editingId])

  const getProjectCount = (wsId: string) =>
    projects.filter((p) => (p.workspaceId ?? 'personal') === wsId).length

  const handleCreate = () => {
    if (!formName.trim()) return
    const id = crypto.randomUUID()
    const maxOrder = Math.max(0, ...workspaces.map((ws) => ws.order))
    addWorkspace({
      id,
      name: formName.trim(),
      icon: formIcon,
      iconColor: formColor,
      order: maxOrder + 1
    })
    setActiveWorkspace(id)
    toast.success(`Workspace "${formName.trim()}" created`)
    resetForm()
  }

  const handleUpdate = () => {
    if (!editingId || !formName.trim()) return
    updateWorkspace(editingId, {
      name: formName.trim(),
      icon: formIcon,
      iconColor: formColor
    })
    toast.success(`Workspace updated`)
    resetForm()
  }

  const handleDelete = (id: string) => {
    if (id === 'personal') return
    const ws = workspaces.find((w) => w.id === id)
    removeWorkspace(id)
    toast.success(`Workspace "${ws?.name}" deleted`)
    setConfirmDeleteId(null)
  }

  const startEdit = (ws: WorkspaceConfig) => {
    setEditingId(ws.id)
    setFormName(ws.name)
    setFormIcon(ws.icon || 'User')
    setFormColor(ws.iconColor || '#6b7280')
    setIsCreating(false)
    setShowIconPicker(false)
  }

  const startCreate = () => {
    setIsCreating(true)
    setEditingId(null)
    setFormName('')
    setFormIcon('Briefcase')
    setFormColor('#3b82f6')
    setShowIconPicker(false)
  }

  return (
    <div className="relative">
      {/* Trigger button */}
      <button
        onClick={() => {
          setIsOpen(!isOpen)
          if (isOpen) resetForm()
        }}
        className="flex items-center gap-2 px-2.5 py-1.5 rounded-md text-[13px]
                   text-gray-200 hover:bg-white/[0.06] transition-colors w-full"
      >
        <WorkspaceIcon icon={currentWorkspace.icon} color={currentWorkspace.iconColor} size={14} />
        <span className="truncate font-medium">{currentWorkspace.name}</span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`text-gray-500 ml-auto shrink-0 transition-transform ${isOpen ? 'rotate-180' : ''}`}
        />
      </button>

      {/* Backdrop — titlebar-no-drag so clicks on Electron drag regions still close */}
      {isOpen && (
        <div
          className="fixed inset-0 z-50 titlebar-no-drag"
          onClick={() => {
            setIsOpen(false)
            resetForm()
          }}
        />
      )}

      {/* Dropdown */}
      {isOpen && (
        <div
          className="absolute left-0 top-full mt-1 z-50 w-[260px] py-1
                     border border-white/[0.08] rounded-lg shadow-xl overflow-hidden"
          style={{ background: 'var(--color-surface-overlay)' }}
        >
          {/* Workspace list */}
          <div className="max-h-[300px] overflow-auto">
            {workspaces.map((ws) => {
              const isActive = ws.id === activeWorkspace
              const count = getProjectCount(ws.id)

              if (editingId === ws.id) {
                return (
                  <div key={ws.id} className="px-2 py-1.5">
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => setShowIconPicker(!showIconPicker)}
                        className="shrink-0 p-1 rounded hover:bg-white/[0.08]"
                      >
                        <WorkspaceIcon icon={formIcon} color={formColor} size={14} />
                      </button>
                      <input
                        ref={inputRef}
                        type="text"
                        value={formName}
                        onChange={(e) => setFormName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleUpdate()
                          if (e.key === 'Escape') resetForm()
                        }}
                        className="flex-1 bg-white/[0.06] border border-white/[0.1] rounded px-2 py-1
                                   text-[13px] text-gray-200 focus:outline-none focus:border-white/[0.2] min-w-0"
                      />
                      <button
                        onClick={handleUpdate}
                        className="shrink-0 p-1 text-green-400 hover:text-green-300"
                      >
                        <Check size={14} strokeWidth={2} />
                      </button>
                      <button
                        onClick={resetForm}
                        className="shrink-0 p-1 text-gray-500 hover:text-gray-300"
                      >
                        <X size={14} strokeWidth={2} />
                      </button>
                    </div>
                    {showIconPicker && (
                      <div className="mt-2 space-y-2">
                        <div className="grid grid-cols-8 gap-1">
                          {WORKSPACE_ICON_OPTIONS.map((opt) => {
                            const IconComp = ICON_MAP[opt.name] || User
                            return (
                              <button
                                key={opt.name}
                                onClick={() => setFormIcon(opt.name)}
                                className={`p-1.5 rounded ${
                                  formIcon === opt.name
                                    ? 'bg-white/[0.1] ring-1 ring-white/[0.2]'
                                    : 'hover:bg-white/[0.06]'
                                }`}
                                title={opt.label}
                              >
                                <IconComp
                                  size={12}
                                  color={formIcon === opt.name ? formColor : '#9ca3af'}
                                  strokeWidth={1.5}
                                />
                              </button>
                            )
                          })}
                        </div>
                        <div className="flex gap-1.5">
                          {ICON_COLOR_PALETTE.map((color) => (
                            <button
                              key={color}
                              onClick={() => setFormColor(color)}
                              className={`w-5 h-5 rounded-full border ${
                                formColor === color
                                  ? 'border-white scale-110'
                                  : 'border-transparent hover:border-white/30'
                              }`}
                              style={{ backgroundColor: color }}
                            />
                          ))}
                        </div>
                      </div>
                    )}
                  </div>
                )
              }

              return (
                <div
                  key={ws.id}
                  className={`group flex items-center gap-2 px-2.5 py-2 cursor-pointer transition-colors ${
                    isActive ? 'bg-white/[0.08] text-white' : 'text-gray-300 hover:bg-white/[0.04]'
                  }`}
                >
                  <button
                    className="flex-1 flex items-center gap-2 min-w-0"
                    onClick={() => {
                      setActiveWorkspace(ws.id)
                      setIsOpen(false)
                      resetForm()
                    }}
                  >
                    <WorkspaceIcon icon={ws.icon} color={ws.iconColor} size={14} />
                    <span className="truncate text-[13px]">{ws.name}</span>
                    <span className="text-[11px] text-gray-600 ml-auto shrink-0">{count}</span>
                  </button>

                  {/* Edit/Delete actions */}
                  <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        startEdit(ws)
                      }}
                      className="p-0.5 text-gray-500 hover:text-gray-300 rounded"
                      title="Edit"
                    >
                      <Pencil size={11} strokeWidth={1.5} />
                    </button>
                    {ws.id !== 'personal' && (
                      <>
                        {confirmDeleteId === ws.id ? (
                          <button
                            onClick={(e) => {
                              e.stopPropagation()
                              handleDelete(ws.id)
                            }}
                            className="p-0.5 text-red-400 hover:text-red-300 rounded text-[10px]"
                            title="Confirm delete"
                          >
                            <Trash2 size={11} strokeWidth={2} />
                          </button>
                        ) : (
                          <button
                            onClick={(e) => {
                              e.stopPropagation()
                              setConfirmDeleteId(ws.id)
                            }}
                            className="p-0.5 text-gray-500 hover:text-red-400 rounded"
                            title="Delete"
                          >
                            <Trash2 size={11} strokeWidth={1.5} />
                          </button>
                        )}
                      </>
                    )}
                  </div>
                </div>
              )
            })}
          </div>

          {/* Divider */}
          <div className="border-t border-white/[0.06] my-1" />

          {/* Create new workspace */}
          {isCreating ? (
            <div className="px-2 py-1.5">
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setShowIconPicker(!showIconPicker)}
                  className="shrink-0 p-1 rounded hover:bg-white/[0.08]"
                >
                  <WorkspaceIcon icon={formIcon} color={formColor} size={14} />
                </button>
                <input
                  ref={inputRef}
                  type="text"
                  value={formName}
                  onChange={(e) => setFormName(e.target.value)}
                  placeholder="Workspace name..."
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') handleCreate()
                    if (e.key === 'Escape') resetForm()
                  }}
                  className="flex-1 bg-white/[0.06] border border-white/[0.1] rounded px-2 py-1
                             text-[13px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-white/[0.2] min-w-0"
                />
                <button
                  onClick={handleCreate}
                  disabled={!formName.trim()}
                  className="shrink-0 p-1 text-green-400 hover:text-green-300 disabled:opacity-30"
                >
                  <Check size={14} strokeWidth={2} />
                </button>
                <button
                  onClick={resetForm}
                  className="shrink-0 p-1 text-gray-500 hover:text-gray-300"
                >
                  <X size={14} strokeWidth={2} />
                </button>
              </div>
              {showIconPicker && (
                <div className="mt-2 space-y-2">
                  <div className="grid grid-cols-8 gap-1">
                    {WORKSPACE_ICON_OPTIONS.map((opt) => {
                      const IconComp = ICON_MAP[opt.name] || User
                      return (
                        <button
                          key={opt.name}
                          onClick={() => setFormIcon(opt.name)}
                          className={`p-1.5 rounded ${
                            formIcon === opt.name
                              ? 'bg-white/[0.1] ring-1 ring-white/[0.2]'
                              : 'hover:bg-white/[0.06]'
                          }`}
                          title={opt.label}
                        >
                          <IconComp
                            size={12}
                            color={formIcon === opt.name ? formColor : '#9ca3af'}
                            strokeWidth={1.5}
                          />
                        </button>
                      )
                    })}
                  </div>
                  <div className="flex gap-1.5">
                    {ICON_COLOR_PALETTE.map((color) => (
                      <button
                        key={color}
                        onClick={() => setFormColor(color)}
                        className={`w-5 h-5 rounded-full border ${
                          formColor === color
                            ? 'border-white scale-110'
                            : 'border-transparent hover:border-white/30'
                        }`}
                        style={{ backgroundColor: color }}
                      />
                    ))}
                  </div>
                </div>
              )}
            </div>
          ) : (
            <button
              onClick={startCreate}
              className="w-full flex items-center gap-2 px-3 py-2 text-[13px] text-gray-400
                         hover:text-gray-200 hover:bg-white/[0.04] transition-colors"
            >
              <Plus size={14} strokeWidth={1.5} />
              Create Workspace
            </button>
          )}
        </div>
      )}
    </div>
  )
}
