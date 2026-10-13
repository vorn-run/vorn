import {
  Zap,
  Clock,
  Calendar,
  ListPlus,
  ArrowRightLeft,
  Plug,
  Globe,
  type LucideIcon,
  RotateCcw
} from 'lucide-react'
import type {
  TriggerConfig,
  ConnectorPollTriggerConfig,
  SdkConnectorIcon
} from '../../../../shared/types'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { ConnectorIcon } from '../../ConnectorIcon'
import { NODE_GLYPH } from '../node-visuals'
import { NodeShell } from './NodeShell'

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
  sessionRestored: RotateCcw,
  connectorPoll: Plug,
  webhook: Globe
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
    case 'sessionRestored':
      return `${config.restore === 'any' ? 'cold or warm' : 'cold'}${config.projectFilter ? ` · ${config.projectFilter}` : ' · any project'}`
    case 'connectorPoll':
      return `${config.event} · ${config.cron}`
    case 'webhook':
      return `${config.method} · this machine`
  }
}

export function TriggerNode({ label, config, selected, onClick }: Props) {
  const Icon = TRIGGER_ICONS[config.triggerType] || DEFAULT_ICON
  const { connectorId, icon: connectorGlyph } = useConnectorGlyph(config)

  return (
    <NodeShell
      icon={
        connectorId ? (
          <ConnectorIcon
            connectorId={connectorId}
            icon={connectorGlyph}
            size={18}
            className="text-ink shrink-0"
          />
        ) : (
          <Icon size={18} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
        )
      }
      label={label}
      subtitle={getSubtitle(config)}
      selected={selected}
      onClick={onClick}
    />
  )
}
