import { useEffect, useState } from 'react'
import type {
  ConnectorPollTriggerConfig,
  SourceConnection,
  ConnectorManifest
} from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import { ConnectorIcon } from '../../ConnectorIcon'
import { connectionIcon } from '../../../lib/connection-icon'

interface Props {
  config: ConnectorPollTriggerConfig
  onChange: (config: ConnectorPollTriggerConfig) => void
}

interface ConnectorInfo {
  id: string
  name: string
  icon: string
  capabilities: string[]
  manifest: ConnectorManifest
}

export function ConnectorPollTriggerForm({ config, onChange }: Props) {
  const [connectors, setConnectors] = useState<ConnectorInfo[]>([])
  const [connections, setConnections] = useState<SourceConnection[]>([])

  useEffect(() => {
    window.api.listConnectors().then(setConnectors)
    window.api.listConnections().then(setConnections)
  }, [])

  const selectedConn = connections.find((c) => c.id === config.connectionId)
  const selectedConnector = selectedConn
    ? connectors.find((c) => c.id === selectedConn.connectorId)
    : undefined

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Connection</label>
        <SelectPicker
          value={config.connectionId}
          options={connections.map((c) => ({
            value: c.id,
            label: c.name,
            icon: (
              <ConnectorIcon
                connectorId={c.connectorId}
                icon={connectionIcon(c)}
                size={14}
                className="text-gray-400"
              />
            )
          }))}
          onChange={(v) => onChange({ ...config, connectionId: v, event: '' })}
          variant="form"
          placeholder="Select a connection..."
        />
        {connections.length === 0 && (
          <p className="text-[11px] text-gray-500 mt-1.5">
            No connections yet. Add one from Settings › Connectors.
          </p>
        )}
      </div>

      {selectedConnector && (
        <div>
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Event</label>
          <SelectPicker
            value={config.event}
            options={(selectedConnector.manifest.triggers ?? []).map((t) => ({
              value: t.type,
              label: t.label
            }))}
            onChange={(v) => onChange({ ...config, event: v })}
            variant="form"
            placeholder="Select an event..."
          />
        </div>
      )}

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Cron</label>
        <input
          type="text"
          value={config.cron}
          onChange={(e) => onChange({ ...config, cron: e.target.value })}
          placeholder="*/5 * * * *"
          className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-sm
                     text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2] font-mono"
        />
        <p className="text-[11px] text-gray-500 mt-1">
          min hour day month weekday · default every 5 minutes
        </p>
      </div>
    </div>
  )
}
