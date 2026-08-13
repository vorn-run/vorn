import { Zap, Clock, Calendar, ListPlus, ArrowRightLeft, Plug, type LucideIcon } from 'lucide-react'
import type {
  TriggerConfig,
  ConnectorPollTriggerConfig,
  SdkConnectorIcon
} from '../../../../shared/types'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { ConnectorIcon } from '../../ConnectorIcon'
import { NODE_SELECTED, NODE_UNSELECTED, NODE_GLYPH } from '../node-visuals'

interface Props {
  label: string
  config: TriggerConfig
  selected?: boolean
  onClick: () => void
}

const TRIGGER_ICONS: Record<string, LucideIcon> = {
  manual: Zap,
  once: Calendar,
  recurring: Clock,
  taskCreated: ListPlus,
  taskStatusChanged: ArrowRightLeft,
  connectorPoll: Plug
}
const DEFAULT_ICON = Zap

function useConnectorGlyph(config: TriggerConfig): {
  connectorId: string | null
  icon: SdkConnectorIcon | undefined
} {
  // For connectorPoll triggers, resolve the connector via the shared
  // connections cache — avoids one IPC call per node instance.
  const connectionId =
    config.triggerType === 'connectorPoll'
      ? (config as ConnectorPollTriggerConfig).connectionId
      : null
  return {
    connectorId: useConnectorIdFor(connectionId),
    icon: useConnectionIconFor(connectionId)
  }
}

function getSubtitle(config: TriggerConfig): string {
  switch (config.triggerType) {
    case 'manual':
      return 'Click to run'
    case 'once':
      return `Run at ${new Date(config.runAt).toLocaleString()}`
    case 'recurring':
      return `Cron: ${config.cron}`
    case 'taskCreated':
      return config.projectFilter ? `Project: ${config.projectFilter}` : 'Any project'
    case 'taskStatusChanged': {
      const parts: string[] = []
      if (config.fromStatus) parts.push(config.fromStatus)
      if (config.toStatus) parts.push(config.toStatus)
      const transition = parts.length === 2 ? `${parts[0]} → ${parts[1]}` : parts[0] || 'Any change'
      const project = config.projectFilter ? ` · ${config.projectFilter}` : ''
      return transition + project
    }
    case 'connectorPoll':
      return `${config.event} · ${config.cron}`
  }
}

export function TriggerNode({ label, config, selected, onClick }: Props) {
  const Icon = TRIGGER_ICONS[config.triggerType] || DEFAULT_ICON
  const { connectorId, icon: connectorGlyph } = useConnectorGlyph(config)

  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`px-3 py-2.5 rounded-md border w-[280px] transition-all cursor-pointer
                  ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                  bg-surface-node hover:bg-white/[0.02]`}
    >
      <div className="flex items-center gap-2">
        {connectorId ? (
          <ConnectorIcon
            connectorId={connectorId}
            icon={connectorGlyph}
            size={14}
            className="text-gray-400 shrink-0"
          />
        ) : (
          <Icon size={14} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
        )}
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">{getSubtitle(config)}</div>
        </div>
      </div>
    </div>
  )
}
