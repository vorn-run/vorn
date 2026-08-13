import type { ConditionOperator, LoopConfig } from '../../../../shared/types'

const MAX_ITERATIONS = 10

const OPERATORS: { value: ConditionOperator; label: string }[] = [
  { value: 'equals', label: 'equals' },
  { value: 'notEquals', label: 'does not equal' },
  { value: 'contains', label: 'contains' },
  { value: 'notContains', label: 'does not contain' },
  { value: 'isEmpty', label: 'is empty' },
  { value: 'isNotEmpty', label: 'is not empty' }
]

interface Props {
  config: LoopConfig
  onChange: (config: LoopConfig) => void
}

/**
 * Two fields, deliberately.
 *
 * Which steps a loop repeats is answered on the canvas, by putting them inside
 * it. This panel used to carry a checkbox list of every downstream step, which
 * made a spatial relationship into a data-entry form and let the list drift
 * from the graph. Membership has one source of truth now, and it is not here.
 */
export function LoopConfigForm({ config, onChange }: Props) {
  const until = config.until

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Maximum passes</label>
        <input
          type="number"
          min={1}
          max={MAX_ITERATIONS}
          value={config.maxIterations ?? 1}
          onChange={(e) => {
            const n = Number(e.target.value)
            const clamped = Number.isFinite(n)
              ? Math.min(Math.max(1, Math.floor(n)), MAX_ITERATIONS)
              : 1
            onChange({ ...config, maxIterations: clamped })
          }}
          className="w-full px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                     text-[13px] text-gray-200 focus:outline-none focus:border-white/[0.2]"
        />
        <div className="mt-1.5 text-[11px] text-gray-500">
          This is what ends the loop, not a safety net: an agent asked whether its own work is good
          enough tends to say yes. Capped at {MAX_ITERATIONS}.
        </div>
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">
          Stop early when <span className="text-gray-600">(optional)</span>
        </label>
        <div className="space-y-2">
          <input
            type="text"
            value={until?.variable ?? ''}
            onChange={(e) =>
              onChange({
                ...config,
                until: {
                  variable: e.target.value,
                  operator: until?.operator ?? 'equals',
                  value: until?.value ?? ''
                }
              })
            }
            placeholder="{{steps.review.approved}}"
            className="w-full px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                       text-[13px] text-gray-200 placeholder:text-gray-600 font-mono
                       focus:outline-none focus:border-white/[0.2]"
          />
          <div className="flex gap-2">
            <select
              value={until?.operator ?? 'equals'}
              onChange={(e) =>
                onChange({
                  ...config,
                  until: {
                    variable: until?.variable ?? '',
                    operator: e.target.value as ConditionOperator,
                    value: until?.value ?? ''
                  }
                })
              }
              className="px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                         text-[13px] text-gray-200 focus:outline-none focus:border-white/[0.2]"
            >
              {OPERATORS.map((op) => (
                <option key={op.value} value={op.value}>
                  {op.label}
                </option>
              ))}
            </select>
            <input
              type="text"
              value={until?.value ?? ''}
              onChange={(e) =>
                onChange({
                  ...config,
                  until: {
                    variable: until?.variable ?? '',
                    operator: until?.operator ?? 'equals',
                    value: e.target.value
                  }
                })
              }
              placeholder="true"
              className="flex-1 px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                         text-[13px] text-gray-200 placeholder:text-gray-600
                         focus:outline-none focus:border-white/[0.2]"
            />
          </div>
        </div>
        <div className="mt-1.5 text-[11px] text-gray-500">
          Checked after each pass, against a step's typed output. Leave blank to always run the
          maximum.
        </div>
      </div>
    </div>
  )
}
