import { Repeat } from 'lucide-react'
import type { LoopConfig, NodeExecutionStatus, WorkflowNode } from '../../../../shared/types'
import { NODE_GLYPH } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

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
    <NodeShell
      icon={<Repeat size={14} className={`shrink-0 ${NODE_GLYPH}`} strokeWidth={2} />}
      label={label}
      subtitle={subtitle}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
      dashed={body.length === 0}
      trailing={
        iteration !== undefined &&
        iteration > 0 && (
          <span className="shrink-0 text-[10px] text-gray-400 bg-white/[0.06] rounded px-1.5 py-0.5">
            {iteration}×
          </span>
        )
      }
    >
      {body.length > 0 && (
        <NodeFooter rows>
          {body.map((n, i) => (
            <div key={n.id} className="text-[11px] text-gray-500 truncate">
              <span className="text-gray-600 tabular-nums">{i + 1}.</span> {n.label}
            </div>
          ))}
        </NodeFooter>
      )}

      {config.until?.variable && (
        <NodeFooter>
          until {config.until.variable} {config.until.operator} {config.until.value}
        </NodeFooter>
      )}
    </NodeShell>
  )
}
