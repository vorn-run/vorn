import type { OutlineStep } from '../../lib/workflow-helpers'
import { NODE_TYPE_ICON } from './node-visuals'

interface Props {
  steps: OutlineStep[]
  /** The step the arrow keys are on, or the selected one. */
  focusedId: string | null
  /** Steps the canvas is showing right now. */
  visibleIds: ReadonlySet<string>
  onFocus: (nodeId: string) => void
}

/** Every step of the workflow in the order it runs, so a long one can be crossed without zooming out. */
export function StepOutline({ steps, focusedId, visibleIds, onFocus }: Props) {
  return (
    <nav
      aria-label="Steps"
      className="w-[200px] shrink-0 h-full flex flex-col border-r border-white/[0.06] bg-surface-panel"
    >
      <div className="px-3.5 pt-3 pb-2 text-[10px] font-mono uppercase tracking-wider text-ink-faint">
        Steps · {steps.length}
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto px-2 flex flex-col">
        {steps.map(({ node, depth }) => {
          const Icon = NODE_TYPE_ICON[node.type]
          const focused = node.id === focusedId
          const tone = focused
            ? 'bg-white/[0.05] text-ink'
            : visibleIds.has(node.id)
              ? 'text-ink hover:bg-white/[0.03]'
              : 'text-ink-faint hover:text-ink-secondary hover:bg-white/[0.03]'
          return (
            <button
              key={node.id}
              type="button"
              onClick={() => onFocus(node.id)}
              aria-current={focused ? 'step' : undefined}
              className={`shrink-0 h-[26px] flex items-center gap-2 rounded pr-1.5 text-left text-[12px] transition-colors ${tone}`}
              style={{ paddingLeft: 6 + depth * 16 }}
            >
              <Icon size={12} className="shrink-0" />
              <span className="truncate">{node.label}</span>
            </button>
          )
        })}
      </div>
      <div className="px-3.5 py-3 flex items-center gap-1.5 text-[11px] text-ink-faint">
        <kbd className="font-mono text-[10px] border border-white/[0.08] rounded px-1">↑</kbd>
        <kbd className="font-mono text-[10px] border border-white/[0.08] rounded px-1">↓</kbd>
        move between steps
      </div>
    </nav>
  )
}
