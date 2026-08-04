import { ConditionConfig, ConditionOperator, TriggerConfig } from '../../../../shared/types'
import { TEMPLATE_VARIABLES, StepVariableGroup, TemplateVariable } from '../../../lib/template-vars'
import { VariableAutocomplete } from './VariableAutocomplete'
import { SelectPicker } from '../../SelectPicker'

interface Props {
  config: ConditionConfig
  onChange: (config: ConditionConfig) => void
  triggerType?: TriggerConfig['triggerType']
  inputVars?: TemplateVariable[]
  stepGroups: StepVariableGroup[]
}

const OPERATORS = [
  { value: 'equals', label: 'equals' },
  { value: 'notEquals', label: 'not equals' },
  { value: 'contains', label: 'contains' },
  { value: 'notContains', label: 'not contains' },
  { value: 'isEmpty', label: 'is empty' },
  { value: 'isNotEmpty', label: 'is not empty' }
]

const hiddenValueOperators: ConditionOperator[] = ['isEmpty', 'isNotEmpty']

export function ConditionConfigForm({
  config,
  onChange,
  triggerType,
  inputVars = [],
  stepGroups
}: Props) {
  const contextVars = TEMPLATE_VARIABLES.filter((v) => {
    if (v.category === 'task') {
      return triggerType === 'taskCreated' || triggerType === 'taskStatusChanged'
    }
    if (v.category === 'trigger') {
      return triggerType === 'taskStatusChanged'
    }
    return false
  }).concat(inputVars)

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Variable</label>
        <VariableAutocomplete
          value={config.variable || ''}
          onChange={(val) => onChange({ ...config, variable: val })}
          placeholder="e.g. {{steps.build.status}}"
          rows={1}
          stepGroups={stepGroups}
          contextVars={contextVars}
          mono
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Operator</label>
        <SelectPicker
          value={config.operator}
          options={OPERATORS}
          onChange={(v) => onChange({ ...config, operator: v as ConditionOperator })}
          variant="form"
        />
      </div>

      {!hiddenValueOperators.includes(config.operator) && (
        <div>
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Value</label>
          <VariableAutocomplete
            value={config.value || ''}
            onChange={(val) => onChange({ ...config, value: val })}
            placeholder="Compare against..."
            rows={1}
            stepGroups={stepGroups}
            contextVars={contextVars}
            mono
          />
        </div>
      )}

      {config.variable && (
        <div className="px-3 py-2 rounded-lg bg-white/[0.02] border border-white/[0.04]">
          <div className="text-[11px] text-gray-500 uppercase tracking-wider mb-1">Evaluates</div>
          <div className="text-[11px] text-gray-400 font-mono truncate">
            {config.variable} {OPERATORS.find((o) => o.value === config.operator)?.label || ''}
            {!hiddenValueOperators.includes(config.operator) && config.value
              ? ` "${config.value}"`
              : ''}
          </div>
        </div>
      )}
    </div>
  )
}
