import type { ConditionOperator, LoopConfig, WorkflowNode } from '../../../../shared/types'

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
  /** Steps this loop is allowed to repeat: everything downstream of it. */
  candidates: WorkflowNode[]
  onChange: (config: LoopConfig) => void
}

export function LoopConfigForm({ config, candidates, onChange }: Props) {
  const body = config.bodyNodeIds ?? []
  const until = config.until

  const toggle = (id: string): void => {
    // Kept in the candidates' own order rather than click order, so the body
    // always reads in the order the steps will actually run.
    const next = body.includes(id) ? body.filter((b) => b !== id) : [...body, id]
    const ordered = candidates.filter((n) => next.includes(n.id)).map((n) => n.id)
    onChange({ ...config, bodyNodeIds: ordered })
  }

  const gateInBody = candidates.some((n) => body.includes(n.id) && n.type === 'approval')

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Steps to repeat</label>
        {candidates.length === 0 ? (
          <div className="text-[11px] text-gray-500">
            Add steps after this loop first, then choose which of them it repeats.
          </div>
        ) : (
          <div className="space-y-1">
            {candidates.map((node) => (
              <label
                key={node.id}
                className="flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer
                           hover:bg-white/[0.03] transition-colors"
              >
                <input
                  type="checkbox"
                  checked={body.includes(node.id)}
                  onChange={() => toggle(node.id)}
                  className="accent-cyan-500"
                />
                <span className="text-[13px] text-gray-200 truncate">{node.label}</span>
                <span className="text-[11px] text-gray-600 shrink-0">{node.type}</span>
              </label>
            ))}
          </div>
        )}
        {gateInBody && (
          <div className="mt-2 text-[11px] text-amber-400/90">
            An approval gate cannot be repeated: the run would park mid-pass with no way to resume
            at the right one. Remove it from the body.
          </div>
        )}
      </div>

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
                     text-[13px] text-gray-200 focus:outline-none focus:border-blue-500/50"
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
                       focus:outline-none focus:border-blue-500/50"
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
                         text-[13px] text-gray-200 focus:outline-none focus:border-blue-500/50"
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
                         focus:outline-none focus:border-blue-500/50"
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
