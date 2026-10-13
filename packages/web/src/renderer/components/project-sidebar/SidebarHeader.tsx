import { isMac, isWeb, MOD, TRAFFIC_LIGHT_PAD_PX } from '../../lib/platform'
import { useAppStore } from '../../stores'
import { WorkspaceSwitcher } from '../WorkspaceSwitcher'
import { PanelLeft, Monitor, ListTodo, Workflow } from 'lucide-react'
import { Tooltip } from '../Tooltip'

const VIEW_MODES = [
  { mode: 'sessions', label: 'Sessions', icon: Monitor, shortcutKey: 'S' },
  { mode: 'tasks', label: 'Tasks', icon: ListTodo, shortcutKey: 'T' },
  { mode: 'workflows', label: 'Workflows', icon: Workflow, shortcutKey: '⇧W' }
] as const

export function SidebarHeader({ isCollapsed }: { isCollapsed: boolean }) {
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  const mainViewMode = useAppStore((s) => s.config?.defaults?.mainViewMode ?? 'sessions')
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)

  return (
    <div className="shrink-0 border-b border-white/[0.06]">
      <div
        className="titlebar-drag h-[40px] pr-3 flex items-center"
        style={
          isMac && !isWeb ? { paddingLeft: `${TRAFFIC_LIGHT_PAD_PX}px` } : { paddingLeft: '12px' }
        }
      >
        {!isCollapsed && (
          <div className="flex-1 titlebar-no-drag min-w-0">
            <WorkspaceSwitcher />
          </div>
        )}
        {!isCollapsed && (
          <button
            onClick={toggleSidebar}
            className="text-gray-400 hover:text-white titlebar-no-drag p-1 rounded-md transition-colors shrink-0"
          >
            <PanelLeft size={16} strokeWidth={2} />
          </button>
        )}
      </div>

      <div
        className={`titlebar-no-drag flex items-center gap-1 py-2 ${
          isCollapsed ? 'flex-col justify-center px-1.5' : 'px-3'
        }`}
      >
        {VIEW_MODES.map(({ mode, label, icon: Icon, shortcutKey }) => {
          const isActive = mainViewMode === mode
          return (
            <Tooltip
              key={mode}
              label={label}
              shortcut={`${MOD}${shortcutKey}`}
              position={isCollapsed ? 'right' : 'bottom'}
            >
              <button
                onClick={() => setMainViewMode(mode)}
                aria-label={label}
                aria-pressed={isActive}
                className={`flex items-center gap-1.5 rounded-lg text-[12px] font-medium transition-colors ${
                  isCollapsed ? 'p-2' : 'px-2.5 py-1.5'
                } ${
                  isActive
                    ? 'bg-white/[0.1] text-white'
                    : 'text-gray-500 hover:text-gray-300 hover:bg-white/[0.04]'
                }`}
              >
                <Icon size={14} strokeWidth={2} />
                {!isCollapsed && isActive && label}
              </button>
            </Tooltip>
          )
        })}
      </div>
    </div>
  )
}
