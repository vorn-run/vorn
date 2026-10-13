import { useState } from 'react'
import { ChevronRight, MoreHorizontal } from 'lucide-react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { InlineRename } from '../InlineRename'
import { ProjectIcon } from './ProjectIcon'
import { GroupContextMenu } from './GroupContextMenu'
import { IconColorPicker } from '../IconColorPicker'
import type { SessionGroupConfig } from '../../../shared/types'

export function SessionGroupItem({
  group,
  sessionCount,
  hasWaiting,
  isExpanded,
  onToggleExpanded,
  startRenaming = false,
  onRenameSettled
}: {
  group: SessionGroupConfig
  sessionCount: number
  hasWaiting: boolean
  isExpanded: boolean
  onToggleExpanded: () => void
  startRenaming?: boolean
  onRenameSettled?: () => void
}) {
  const activeGroupId = useAppStore((s) => s.activeGroupId)
  const setActiveGroup = useAppStore((s) => s.setActiveGroup)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const updateSessionGroup = useAppStore((s) => s.updateSessionGroup)
  const removeSessionGroup = useAppStore((s) => s.removeSessionGroup)
  const terminals = useAppStore((s) => s.terminals)
  const moveSessionToGroup = useAppStore((s) => s.moveSessionToGroup)

  const [renaming, setRenaming] = useState(startRenaming)
  const [openMenu, setOpenMenu] = useState(false)
  const [pickingIcon, setPickingIcon] = useState(false)

  const isActive = activeGroupId === group.id

  if (renaming) {
    return (
      <div className="flex items-center gap-2 px-2 py-1.5 min-w-0">
        <ProjectIcon icon={group.icon} color={group.iconColor} size={14} />
        <InlineRename
          value={group.name}
          onCommit={(next) => {
            if (next !== group.name) updateSessionGroup(group.id, { name: next })
            setRenaming(false)
            onRenameSettled?.()
          }}
          onCancel={() => {
            setRenaming(false)
            onRenameSettled?.()
          }}
          className="flex-1 min-w-0 text-[13px]"
        />
      </div>
    )
  }

  return (
    <div className="relative">
      <div className="group/grp relative flex items-center">
        {/* A div with the button role: the row nests real buttons inside it. */}
        <div
          role="button"
          tabIndex={0}
          aria-pressed={isActive}
          aria-label={group.name}
          onKeyDown={(e) => {
            // Keys bubbling from a nested control are that control's, not the row's.
            if (e.target !== e.currentTarget) return
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              e.currentTarget.click()
            }
          }}
          onClick={() => {
            setActiveGroup(isActive ? null : group.id)
            setFocusedTerminal(null)
          }}
          className={`flex-1 cursor-pointer select-none text-left px-2 py-1.5 rounded-md text-[13px] transition-colors flex items-center gap-2 min-w-0 ${
            isActive
              ? 'bg-white/[0.08] text-white'
              : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
          }`}
        >
          {/* The icon becomes the disclosure on hover, the way a project row
              does it — one glyph, not two. */}
          <div
            role="button"
            tabIndex={0}
            aria-expanded={isExpanded}
            aria-label={`Toggle ${group.name}`}
            className="relative w-[14px] h-[14px] shrink-0"
            onClick={(e) => {
              e.stopPropagation()
              onToggleExpanded()
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.stopPropagation()
                e.preventDefault()
                onToggleExpanded()
              }
            }}
          >
            <span className="group-hover/grp:hidden flex items-center justify-center w-full h-full">
              <ProjectIcon icon={group.icon} color={group.iconColor} size={14} />
            </span>
            <ChevronRight
              size={12}
              strokeWidth={2.5}
              className={`hidden group-hover/grp:block text-gray-500 transition-transform absolute top-[1px] left-[1px] ${
                isExpanded ? 'rotate-90' : ''
              }`}
            />
          </div>
          <span className="truncate">{group.name}</span>
          {/* Shut, the group answers for the sessions it hides. */}
          {hasWaiting && !isExpanded && (
            <span
              aria-label="A session in this group is waiting"
              className="ml-auto w-[5px] h-[5px] rounded-full bg-bronzo shrink-0"
            />
          )}
          {sessionCount > 0 && (
            <span
              className={`text-gray-500 text-xs shrink-0 group-hover/grp:hidden ${
                hasWaiting && !isExpanded ? 'ml-1.5' : 'ml-auto'
              }`}
            >
              {sessionCount}
            </span>
          )}
          <div className="hidden group-hover/grp:flex items-center gap-0.5 ml-auto">
            <Tooltip label="More" position="right">
              <button
                type="button"
                aria-label={`More actions for ${group.name}`}
                onClick={(e) => {
                  e.stopPropagation()
                  setOpenMenu(!openMenu)
                }}
                className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors"
              >
                <MoreHorizontal size={14} strokeWidth={2} />
              </button>
            </Tooltip>
          </div>
        </div>
        {openMenu && (
          <div className="relative">
            <GroupContextMenu
              group={group}
              onRename={() => setRenaming(true)}
              onChangeIcon={() => setPickingIcon(true)}
              onUngroup={() => {
                for (const [id, t] of terminals) {
                  if (t.session.groupId === group.id) moveSessionToGroup(id, null)
                }
              }}
              onDelete={() => removeSessionGroup(group.id)}
              onClose={() => setOpenMenu(false)}
            />
          </div>
        )}
      </div>

      {pickingIcon && (
        <div
          className="absolute left-0 top-full mt-1 z-50 w-[248px] p-3
                     bg-surface-overlay border border-white/[0.08] rounded-lg shadow-xl"
        >
          <IconColorPicker
            icon={group.icon ?? 'Folder'}
            color={group.iconColor ?? '#6b7280'}
            onIconChange={(icon) => updateSessionGroup(group.id, { icon })}
            onColorChange={(iconColor) => updateSessionGroup(group.id, { iconColor })}
          />
          <button
            onClick={() => setPickingIcon(false)}
            className="mt-3 w-full px-2 py-1 text-[11px] text-gray-300 bg-white/[0.04]
                       hover:bg-white/[0.08] border border-white/[0.08] rounded transition-colors"
          >
            Done
          </button>
        </div>
      )}
    </div>
  )
}
