import { useEffect, useRef } from 'react'
import { Plus, Check } from 'lucide-react'
import { useAppStore } from '../../stores'
import { useWorkspaceSessionGroups } from '../../hooks/useWorkspaceSessionGroups'
import { ProjectIcon } from './ProjectIcon'

/**
 * A flat section rather than a cascade: the sidebar is 256px, which a flyout
 * cannot open into.
 */
export function SessionContextMenu({
  sessionId,
  currentGroupId,
  onNewGroup,
  onClose
}: {
  sessionId: string
  currentGroupId?: string
  onNewGroup: () => void
  onClose: () => void
}) {
  const menuRef = useRef<HTMLDivElement>(null)
  const groups = useWorkspaceSessionGroups()
  const moveSessionToGroup = useAppStore((s) => s.moveSessionToGroup)

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('pointerdown', handleClickOutside)
    return () => document.removeEventListener('pointerdown', handleClickOutside)
  }, [onClose])

  return (
    <div
      ref={menuRef}
      className="absolute right-0 top-full mt-1 z-50 min-w-[176px] py-1
                 bg-surface-overlay border border-white/[0.08] rounded-lg shadow-xl"
    >
      <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
        Move to group
      </div>
      {groups.map((g) => {
        const selected = currentGroupId === g.id
        return (
          <button
            key={g.id}
            onClick={() => {
              moveSessionToGroup(sessionId, selected ? null : g.id)
              onClose()
            }}
            className={`w-full pl-5 pr-3 py-2 text-left text-[13px] flex items-center gap-2 transition-colors ${
              selected
                ? 'text-white bg-white/[0.06]'
                : 'text-gray-300 hover:text-white hover:bg-white/[0.06]'
            }`}
          >
            <ProjectIcon icon={g.icon} color={g.iconColor} size={12} />
            <span className="truncate">{g.name}</span>
            {selected && <Check size={12} strokeWidth={2.5} className="ml-auto shrink-0" />}
          </button>
        )
      })}
      <button
        onClick={() => {
          onNewGroup()
          onClose()
        }}
        className="w-full pl-5 pr-3 py-2 text-left text-[13px] text-gray-300 hover:text-white
                   hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
      >
        <Plus size={12} strokeWidth={2} />
        New group…
      </button>
      {currentGroupId && (
        <button
          onClick={() => {
            moveSessionToGroup(sessionId, null)
            onClose()
          }}
          className="w-full pl-5 pr-3 py-2 text-left text-[13px] text-gray-300 hover:text-white
                     hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
        >
          Remove from group
        </button>
      )}
    </div>
  )
}
