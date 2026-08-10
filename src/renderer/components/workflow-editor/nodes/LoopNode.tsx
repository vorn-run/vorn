import { Repeat } from 'lucide-react'
import type { LoopConfig, NodeExecutionStatus, WorkflowNode } from '../../../../shared/types'
import { STATUS_DOT_CLASSES } from '../statusDot'

interface Props {
  label: string
  config: LoopConfig
  /** Every node in the workflow, so the card can name the steps it repeats. */
  nodes: WorkflowNode[]
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  /** Passes actually run, once a run has happened. */
  iteration?: number
  onClick: () => void
}

export function LoopNode({
  label,
  config,
  nodes,
  selected,
  executionStatus,
  iteration,
  onClick
}: Props) {
  const body = (config.bodyNodeIds ?? [])
    .map((id) => nodes.find((n) => n.id === id))
    .filter((n): n is WorkflowNode => Boolean(n))

  const max = config.maxIterations ?? 1
  // The body is what makes a loop a loop, so an empty one is worth saying out
  // loud on the card rather than leaving the reader to wonder why nothing
  // repeated.
  const subtitle =
    body.length === 0
      ? 'No steps selected yet'
      : `Repeats ${body.length} step${body.length === 1 ? '' : 's'} · up to ${max}×`

  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`relative px-3 py-2.5 rounded-md border w-[280px] transition-all cursor-pointer
                  ${selected ? 'border-blue-500/60 shadow-[0_0_0_3px_rgba(59,130,246,0.08)]' : 'border-white/[0.08]'}
                  ${body.length === 0 ? 'border-dashed' : ''}
                  bg-[#1d1d20] hover:bg-white/[0.02]`}
    >
      {executionStatus && STATUS_DOT_CLASSES[executionStatus] && (
        <span
          className={`absolute top-2 right-2 w-1.5 h-1.5 rounded-full ${STATUS_DOT_CLASSES[executionStatus]}`}
        />
      )}
      <div className="flex items-center gap-2">
        <Repeat size={14} className="shrink-0 text-cyan-400" strokeWidth={2} />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">{subtitle}</div>
        </div>
        {iteration !== undefined && iteration > 0 && (
          <span className="shrink-0 text-[10px] text-cyan-300/80 bg-cyan-500/10 rounded px-1.5 py-0.5">
            {iteration}×
          </span>
        )}
      </div>

      {body.length > 0 && (
        <div className="mt-2 border-t border-white/[0.06] pt-2 space-y-0.5">
          {body.map((n, i) => (
            <div key={n.id} className="text-[11px] text-gray-500 truncate">
              <span className="text-gray-600 tabular-nums">{i + 1}.</span> {n.label}
            </div>
          ))}
        </div>
      )}

      {config.until?.variable && (
        <div className="mt-2 text-[11px] text-gray-600 truncate border-t border-white/[0.06] pt-2">
          until {config.until.variable} {config.until.operator} {config.until.value}
        </div>
      )}
    </div>
  )
}
