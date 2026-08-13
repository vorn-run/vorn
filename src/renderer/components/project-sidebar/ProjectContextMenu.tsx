import { useState, useEffect, useRef } from 'react'
import { Pencil, Trash2 } from 'lucide-react'
import type { ProjectConfig } from '../../../shared/types'
import { toast } from '../Toast'

export function ProjectContextMenu({
  project,
  onEdit,
  onDelete,
  onClose
}: {
  project: ProjectConfig
  onEdit: () => void
  onDelete: () => void
  onClose: () => void
}) {
  const menuRef = useRef<HTMLDivElement>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose()
      }
    }
    document.addEventListener('pointerdown', handleClickOutside)
    return () => document.removeEventListener('pointerdown', handleClickOutside)
  }, [onClose])

  return (
    <div
      ref={menuRef}
      className="absolute right-0 top-full mt-1 z-50 min-w-[140px] py-1
                 border border-white/[0.08] rounded-lg shadow-xl"
      style={{ background: '#141416' }}
    >
      <button
        onClick={() => {
          onEdit()
          onClose()
        }}
        className="w-full px-3 py-2.5 text-left text-[13px] text-gray-300 hover:text-white
                   hover:bg-white/[0.06] active:bg-white/[0.1] flex items-center gap-2 transition-colors"
      >
        <Pencil size={12} strokeWidth={1.5} />
        Edit Project
      </button>
      {confirmDelete ? (
        <button
          onClick={() => {
            onDelete()
            onClose()
            toast.success(`Project "${project.name}" deleted`)
          }}
          className="w-full px-3 py-2.5 text-left text-[13px] text-danger bg-danger/10
                     hover:bg-danger/20 active:bg-danger/30 flex items-center gap-2 transition-colors"
        >
          <Trash2 size={12} strokeWidth={1.5} />
          Confirm delete?
        </button>
      ) : (
        <button
          onClick={() => setConfirmDelete(true)}
          className="w-full px-3 py-2.5 text-left text-[13px] text-danger/80 hover:text-danger
                     hover:bg-white/[0.06] active:bg-white/[0.1] flex items-center gap-2 transition-colors"
        >
          <Trash2 size={12} strokeWidth={1.5} />
          Delete Project
        </button>
      )}
    </div>
  )
}
