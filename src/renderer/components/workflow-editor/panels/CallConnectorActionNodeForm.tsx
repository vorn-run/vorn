import { useEffect, useState } from 'react'
import type {
  CallConnectorActionConfig,
  ConnectorActionDef,
  ConnectorConfigField,
  TriggerConfig
} from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnections, iconForConnection } from '../../../lib/use-connections'
import { TEMPLATE_VARIABLES, StepVariableGroup, TemplateVariable } from '../../../lib/template-vars'
import { VariableAutocomplete } from './VariableAutocomplete'

interface Props {
  config: CallConnectorActionConfig
  onChange: (config: CallConnectorActionConfig) => void
  triggerType?: TriggerConfig['triggerType']
  inputVars?: TemplateVariable[]
  stepGroups?: StepVariableGroup[]
}

interface ArgumentFieldProps {
  field: ConnectorConfigField
  value: string
  onChange: (value: string) => void
  stepGroups: StepVariableGroup[]
  contextVars: TemplateVariable[]
}

/**
 * One argument's editor.
 *
 * A connector's listed choices are suggestions rather than a closed set: a step
 * is entitled to compute this value from an earlier one, and the connector —
 * not this form — is what finally judges it. So the picker is offered while the
 * value is one of the choices, the template-aware input whenever it is not, and
 * there is a way to move between them on purpose.
 */
function ArgumentField({ field, value, onChange, stepGroups, contextVars }: ArgumentFieldProps) {
  const choices = field.options ?? []
  const listed = choices.some((option) => option.value === value)
  const [chosenFree, setChosenFree] = useState(false)
  // Derived rather than stored, so a value that arrives already holding a
  // template opens in the input that can edit it.
  const free = chosenFree || (value !== '' && !listed)
  const pickable = field.type === 'select' && choices.length > 0

  if (pickable && !free) {
    return (
      <>
        <SelectPicker
          value={value}
          options={choices.map((option) => ({ value: option.value, label: option.label }))}
          onChange={onChange}
          variant="form"
          placeholder={field.placeholder ?? '—'}
        />
        <button
          type="button"
          onClick={() => setChosenFree(true)}
          className="text-[10px] text-gray-500 hover:text-gray-300 mt-1 transition-colors"
        >
          Use a template instead
        </button>
      </>
    )
  }

  return (
    <>
      <VariableAutocomplete
        value={value}
        onChange={onChange}
        placeholder={field.placeholder}
        rows={field.type === 'textarea' ? 3 : 1}
        stepGroups={stepGroups}
        contextVars={contextVars}
        mono
      />
      {pickable && (
        <button
          type="button"
          onClick={() => {
            setChosenFree(false)
            // Cleared, because the list cannot show a value it does not hold.
            if (!listed) onChange('')
          }}
          className="text-[10px] text-gray-500 hover:text-gray-300 mt-1 transition-colors"
        >
          Choose from the list
        </button>
      )}
    </>
  )
}

export function CallConnectorActionNodeForm({
  config,
  onChange,
  triggerType,
  inputVars = [],
  stepGroups = []
}: Props) {
  // The shared cache, so the glyphs here resolve the same way the cards' do.
  const connections = useConnections()
  const [actions, setActions] = useState<ConnectorActionDef[]>([])

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
      if (triggerType === 'webhook') {
        return (
          v.key.includes('trigger.body') ||
          v.key.includes('trigger.headers') ||
          v.key.includes('trigger.query')
        )
      }
      return triggerType === 'taskStatusChanged' && v.key.includes('Status')
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
                icon={iconForConnection(c)}
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
            options={actions.map((a) => ({
              value: a.type,
              label: a.label,
              // The tool name stays visible; it is what a run and its logs name.
              ...(a.label !== a.type && { hint: a.type })
            }))}
            onChange={(v) =>
              onChange({
                ...config,
                action: v,
                // Denormalized so the card can name the action without an IPC call per render.
                actionLabel: actions.find((a) => a.type === v)?.label ?? v,
                args: {}
              })
            }
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
                  {field.required && <span className="text-danger ml-0.5">*</span>}
                </label>
                {/* A picker when the choices fit, the template-aware input
                    otherwise — text, textarea, and a select holding a value
                    its list does not contain all reference ancestor steps
                    with `{{...}}`. */}
                <ArgumentField
                  field={field}
                  value={fieldValue}
                  onChange={setFieldValue}
                  stepGroups={stepGroups}
                  contextVars={contextVars}
                />
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
