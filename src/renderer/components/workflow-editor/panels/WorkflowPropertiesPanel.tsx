import { X } from 'lucide-react'
import { ToggleSwitch } from '../../settings/ToggleSwitch'
import { PropertyRow } from './PropertyRow'
import { formatRelativeTime } from '../../../lib/format-time'
import { STATUS_DOT_CLASSES } from '../statusDot'
import type { WorkflowNode, TriggerConfig, WorkflowExecution } from '../../../../shared/types'

function formatTriggerSummary(node: WorkflowNode | null): string {
  if (!node) return 'None'
  const config = node.config as TriggerConfig
  switch (config.triggerType) {
    case 'manual':
      return 'Manual'
    case 'once':
      return `Once — ${config.runAt ? new Date(config.runAt).toLocaleString() : 'not set'}`
    case 'recurring':
      return `Recurring — ${config.cron || 'not set'}`
    case 'taskCreated':
      return 'Task created'
    case 'taskStatusChanged':
      return 'Task status changed'
    default:
      return 'Unknown'
  }
}

interface Props {
  enabled: boolean
  onEnabledChange: (v: boolean) => void
  staggerDelayMs: number | undefined
  onStaggerChange: (v: number | undefined) => void
  autoCleanupWorktrees: boolean
  onCleanupChange: (v: boolean) => void
  /** True when every LaunchAgent inherits its worktree from context, so
   *  there's no created worktree for cleanup to act on. */
  cleanupDisabled?: boolean
  triggerNode: WorkflowNode | null
  onSelectTrigger: () => void
  lastRun: WorkflowExecution | null
  onClose: () => void
}

export function WorkflowPropertiesPanel({
  enabled,
  onEnabledChange,
  staggerDelayMs,
  onStaggerChange,
  autoCleanupWorktrees,
  onCleanupChange,
  cleanupDisabled = false,
  triggerNode,
  onSelectTrigger,
  lastRun,
  onClose
}: Props) {
  return (
    <div className="w-[340px] border-l border-white/[0.08] bg-[#1e1e22] flex flex-col h-full overflow-hidden titlebar-no-drag">
      <div className="flex items-center justify-between px-5 py-4 border-b border-white/[0.08]">
        <span className="text-[13px] font-medium text-white">Properties</span>
        <button
          onClick={onClose}
          aria-label="Close properties panel"
          className="text-gray-500 hover:text-white p-1 rounded-md transition-colors"
        >
          <X size={14} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-5 space-y-1">
        <PropertyRow label="Status">
          <div className="flex items-center gap-2">
            <span className="text-[12px] text-gray-200">{enabled ? 'Enabled' : 'Disabled'}</span>
            <ToggleSwitch checked={enabled} onChange={onEnabledChange} />
          </div>
        </PropertyRow>

        <PropertyRow label="Stagger delay">
          <input
            type="number"
            min={0}
            value={staggerDelayMs ?? ''}
            onChange={(e) => {
              const raw = e.target.value
              if (raw === '') return onStaggerChange(undefined)
              const parsed = parseInt(raw, 10)
              if (Number.isFinite(parsed) && parsed >= 0) onStaggerChange(parsed)
            }}
            placeholder="0ms"
            aria-label="Stagger delay in milliseconds"
            className="w-[80px] px-2 py-0.5 text-[12px] bg-white/[0.06] border border-white/[0.08] rounded
                       text-gray-200 focus:outline-none focus:border-white/[0.2]"
          />
        </PropertyRow>

        <PropertyRow label="Cleanup worktrees">
          <span
            title={
              cleanupDisabled
                ? 'Cleanup applies only to worktrees this workflow creates. All agents inherit their worktree from context, so there is nothing to clean up.'
                : undefined
            }
            className={cleanupDisabled ? 'opacity-50' : undefined}
          >
            <ToggleSwitch
              checked={cleanupDisabled ? false : autoCleanupWorktrees}
              onChange={cleanupDisabled ? () => {} : onCleanupChange}
            />
          </span>
        </PropertyRow>

        <div className="pt-2" />

        <PropertyRow label="Trigger">
          <button
            onClick={onSelectTrigger}
            className="text-[12px] text-gray-200 hover:text-white transition-colors"
          >
            {formatTriggerSummary(triggerNode)}
          </button>
        </PropertyRow>

        {lastRun && (
          <PropertyRow label="Last run">
            <div className="flex items-center gap-2">
              <span
                className={`w-1.5 h-1.5 rounded-full shrink-0 ${STATUS_DOT_CLASSES[lastRun.status] ?? 'bg-gray-500'}`}
              />
              <span
                className="text-[12px] text-gray-300"
                title={lastRun.startedAt ? new Date(lastRun.startedAt).toLocaleString() : undefined}
              >
                {lastRun.startedAt ? formatRelativeTime(lastRun.startedAt) : 'Unknown'}
              </span>
            </div>
          </PropertyRow>
        )}
      </div>
    </div>
  )
}
