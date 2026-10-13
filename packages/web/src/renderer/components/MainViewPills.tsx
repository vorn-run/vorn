import { Monitor, ListTodo, Workflow } from 'lucide-react'
import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'
import { useWaitingApprovals } from '../hooks/useWaitingApprovals'
import { isMac } from '../lib/platform'

const pillClass = (active: boolean): string =>
  `relative px-2.5 py-1 rounded-md transition-colors ${
    active ? 'bg-white/[0.1] text-white' : 'text-gray-500 hover:text-gray-300'
  }`

export function MainViewPills() {
  const mainViewMode = useAppStore((s) => s.config?.defaults?.mainViewMode ?? 'sessions')
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const waitingCount = useWaitingApprovals().length

  return (
    <div className="flex bg-white/[0.04] rounded-lg p-0.5 gap-0.5">
      <Tooltip label="Sessions" shortcut={`${isMac ? '⌘' : 'Ctrl+'}S`} position="bottom">
        <button
          onClick={() => setMainViewMode('sessions')}
          className={pillClass(mainViewMode === 'sessions')}
          aria-label="Sessions"
          aria-pressed={mainViewMode === 'sessions'}
        >
          <Monitor size={14} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Tasks" shortcut={`${isMac ? '⌘' : 'Ctrl+'}T`} position="bottom">
        <button
          onClick={() => setMainViewMode('tasks')}
          className={pillClass(mainViewMode === 'tasks')}
          aria-label="Tasks"
          aria-pressed={mainViewMode === 'tasks'}
        >
          <ListTodo size={14} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip
        label={waitingCount > 0 ? `Workflows · ${waitingCount} awaiting review` : 'Workflows'}
        shortcut={`${isMac ? '⌘⇧' : 'Ctrl+Shift+'}W`}
        position="bottom"
      >
        <button
          onClick={() => setMainViewMode('workflows')}
          className={pillClass(mainViewMode === 'workflows')}
          aria-label="Workflows"
          aria-pressed={mainViewMode === 'workflows'}
        >
          <Workflow size={14} strokeWidth={2} />
          {waitingCount > 0 && (
            <span
              aria-hidden
              className="absolute -top-0.5 -right-0.5 min-w-[12px] h-[12px] px-[3px]
                         flex items-center justify-center
                         rounded-full bg-bronzo text-surface-base
                         text-[9px] font-mono leading-none tabular-nums"
            >
              {waitingCount > 9 ? '9+' : waitingCount}
            </span>
          )}
        </button>
      </Tooltip>
    </div>
  )
}
