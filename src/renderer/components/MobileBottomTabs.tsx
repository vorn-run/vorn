import { Monitor, ListTodo, Workflow, Settings } from 'lucide-react'
import { useAppStore } from '../stores'

interface Props {
  /** When true (keyboard open), the tab bar is hidden to maximize screen space. */
  hidden?: boolean
}

/**
 * Bottom tab navigation bar for mobile viewports.
 * Provides thumb-friendly access to the main views: Sessions, Tasks, Settings.
 * Hidden when the virtual keyboard is open.
 */
export function MobileBottomTabs({ hidden }: Props) {
  const mainViewMode = useAppStore((s) => s.config?.defaults?.mainViewMode ?? 'sessions')
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen)
  const isSettingsOpen = useAppStore((s) => s.isSettingsOpen)

  if (hidden) return null

  const tabs = [
    {
      id: 'sessions' as const,
      label: 'Sessions',
      icon: Monitor,
      active: mainViewMode === 'sessions' && !isSettingsOpen,
      onTap: () => {
        if (isSettingsOpen) setSettingsOpen(false)
        setMainViewMode('sessions')
      }
    },
    {
      id: 'tasks' as const,
      label: 'Tasks',
      icon: ListTodo,
      active: mainViewMode === 'tasks' && !isSettingsOpen,
      onTap: () => {
        if (isSettingsOpen) setSettingsOpen(false)
        setMainViewMode('tasks')
      }
    },
    {
      id: 'workflows' as const,
      label: 'Workflows',
      icon: Workflow,
      active: mainViewMode === 'workflows' && !isSettingsOpen,
      onTap: () => {
        if (isSettingsOpen) setSettingsOpen(false)
        setMainViewMode('workflows')
      }
    },
    {
      id: 'settings' as const,
      label: 'Settings',
      icon: Settings,
      active: isSettingsOpen,
      onTap: () => setSettingsOpen(!isSettingsOpen)
    }
  ]

  return (
    <nav
      className="fixed left-3 right-3 flex items-stretch rounded-[32px] z-[35]"
      style={{
        bottom: 'calc(12px + var(--safe-bottom, 0px))',
        background: 'var(--glass-bg, var(--color-surface-sunken))',
        backdropFilter: 'var(--glass-blur, none)',
        WebkitBackdropFilter: 'var(--glass-blur, none)',
        boxShadow: 'var(--glass-shadow, none)'
      }}
    >
      {tabs.map((tab) => {
        const Icon = tab.icon
        return (
          <button
            key={tab.id}
            onClick={tab.onTap}
            className={`flex-1 flex flex-col items-center justify-center gap-0.5 py-2.5 min-h-[52px]
                       transition-colors relative rounded-[32px]
                       ${tab.active ? 'text-white' : 'text-gray-500 active:text-gray-300'}`}
            style={tab.active ? { boxShadow: 'var(--glass-shadow-thumb, none)' } : undefined}
          >
            <Icon size={20} strokeWidth={tab.active ? 2.2 : 1.8} />
            <span className="text-[10px] font-medium leading-none">{tab.label}</span>
          </button>
        )
      })}
    </nav>
  )
}
