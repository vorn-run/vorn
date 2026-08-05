import { useEffect, useState } from 'react'
import type {
  CallConnectorActionConfig,
  SourceConnection,
  ConnectorActionDef,
  TriggerConfig
} from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import { ConnectorIcon } from '../../ConnectorIcon'
import { connectionIcon } from '../../../lib/connection-icon'
import { TEMPLATE_VARIABLES, StepVariableGroup, TemplateVariable } from '../../../lib/template-vars'
import { VariableAutocomplete } from './VariableAutocomplete'

interface Props {
  config: CallConnectorActionConfig
  onChange: (config: CallConnectorActionConfig) => void
  triggerType?: TriggerConfig['triggerType']
  inputVars?: TemplateVariable[]
  stepGroups?: StepVariableGroup[]
}

export function CallConnectorActionNodeForm({
  config,
  onChange,
  triggerType,
  inputVars = [],
  stepGroups = []
}: Props) {
  const [connections, setConnections] = useState<SourceConnection[]>([])
  const [actions, setActions] = useState<ConnectorActionDef[]>([])

  useEffect(() => {
    window.api.listConnections().then(setConnections)
  }, [])

  useEffect(() => {
    let cancelled = false
    // Clear immediately so the picker doesn't briefly show the previous
    // connection's actions while the new fetch is in flight.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setActions([])
    if (!config.connectionId) {
      return () => {
        cancelled = true
      }
    }
    window.api.listConnectionActions(config.connectionId).then((next) => {
      if (!cancelled) setActions(next)
    })
    return () => {
      cancelled = true
    }
  }, [config.connectionId])

  const contextVars = TEMPLATE_VARIABLES.filter((v) => {
    if (v.category === 'task') {
      return triggerType === 'taskCreated' || triggerType === 'taskStatusChanged'
    }
    if (v.category === 'connectorItem') {
      return triggerType === 'connectorPoll'
    }
    if (v.category === 'trigger') {
      return triggerType === 'taskStatusChanged'
    }
    return false
  }).concat(inputVars)

  const selectedConn = connections.find((c) => c.id === config.connectionId)
  const selectedAction: ConnectorActionDef | undefined = actions.find(
    (a) => a.type === config.action
  )

  const argFields = selectedAction?.configFields ?? []

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
          onChange={(v) => onChange({ ...config, connectionId: v, action: '', args: {} })}
          variant="form"
          placeholder="Select a connection..."
        />
        {connections.length === 0 && (
          <p className="text-[11px] text-gray-500 mt-1.5">
            No connections yet. Add one from Settings › Connectors first.
          </p>
        )}
      </div>

      {selectedConn && (
        <div>
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Action</label>
          <SelectPicker
            value={config.action}
            options={actions.map((a) => ({ value: a.type, label: a.label }))}
            onChange={(v) => onChange({ ...config, action: v, args: {} })}
            variant="form"
            placeholder={
              actions.length === 0
                ? 'No actions available for this connection'
                : 'Select an action...'
            }
          />
          {selectedAction?.description && (
            <p className="text-[11px] text-gray-500 mt-1.5">{selectedAction.description}</p>
          )}
        </div>
      )}

      {argFields.length > 0 && (
        <div className="space-y-3">
          <div className="text-[11px] text-gray-500 uppercase tracking-wider">
            Arguments{' '}
            <span className="normal-case tracking-normal text-gray-600">
              · type <code className="text-gray-500">{`{{`}</code> to pick from previous steps
            </span>
          </div>
          {argFields.map((field) => {
            const fieldValue =
              typeof config.args?.[field.key] === 'string' ? (config.args[field.key] as string) : ''
            const setFieldValue = (val: string): void =>
              onChange({ ...config, args: { ...config.args, [field.key]: val } })
            return (
              <div key={field.key}>
                <label className="block text-xs text-gray-500 mb-1">
                  {field.label}
                  {field.required && <span className="text-red-400 ml-0.5">*</span>}
                </label>
                {field.type === 'select' ? (
                  <SelectPicker
                    value={fieldValue}
                    options={(field.options ?? []).map((o) => ({
                      value: o.value,
                      label: o.label
                    }))}
                    onChange={setFieldValue}
                    variant="form"
                    placeholder={field.placeholder ?? '—'}
                  />
                ) : (
                  // text / textarea / password / etc. all flow through the
                  // template-aware autocomplete so users can reference
                  // ancestor steps with `{{...}}`.
                  <VariableAutocomplete
                    value={fieldValue}
                    onChange={setFieldValue}
                    placeholder={field.placeholder}
                    rows={field.type === 'textarea' ? 3 : 1}
                    stepGroups={stepGroups}
                    contextVars={contextVars}
                    mono
                  />
                )}
                {field.description && (
                  <p className="text-[10px] text-gray-600 mt-0.5">{field.description}</p>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
