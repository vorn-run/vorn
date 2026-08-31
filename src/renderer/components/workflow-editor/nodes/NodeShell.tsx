import type { ReactNode } from 'react'
import type { NodeExecutionStatus } from '../../../../shared/types'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../../lib/workflow-status'
import { NODE_SELECTED, NODE_UNSELECTED } from '../node-visuals'

/**
 * The card every step on the canvas is drawn in.
 *
 * Eight components repeated this shell almost exactly: the same root classes,
 * the same click handling, the same status dot, the same icon-then-two-lines
 * header. "Almost" is the problem — two had drifted to `rounded-sm` and sat
 * visibly smaller beside the rest on one canvas, and a status dot added to
 * seven of them was missed by the eighth. Each card now says only what makes it
 * that kind of step.
 *
 * The root classes are written here literally rather than composed, for two
 * reasons that point the same way: Tailwind scans source text, so a name built
 * at runtime is never generated; and `node-selection.test.tsx` reads
 * `container.firstChild` as the card root and checks the shape is shared, which
 * only holds while this element is the outermost one.
 */
export function NodeShell({
  icon,
  label,
  subtitle,
  selected,
  executionStatus,
  onClick,
  dashed,
  meta,
  trailing,
  children
}: {
  icon: ReactNode
  label: string
  /** The one line under the label: what this step is pointed at. */
  subtitle: ReactNode
  selected?: boolean
  /**
   * What this node is doing in a live run. Absent on a canvas showing a
   * definition, which is most of the time.
   */
  executionStatus?: NodeExecutionStatus
  onClick: () => void
  /** A loop with no body is an outline of a step rather than a step. */
  dashed?: boolean
  /** Extra rows below the subtitle, inside the text column. */
  meta?: ReactNode
  /** A chip pinned to the right of the header row. */
  trailing?: ReactNode
  /** Footer sections, each its own `NodeFooter`. */
  children?: ReactNode
}) {
  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`relative px-3 py-2.5 rounded-md border w-[280px] transition-all cursor-pointer
                  ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                  ${dashed ? 'border-dashed' : ''}
                  bg-surface-node hover:bg-white/[0.02]`}
    >
      {executionStatus && WORKFLOW_STATUS_DOT_PULSE[executionStatus] && (
        <span
          className={`absolute top-2 right-2 w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT_PULSE[executionStatus]}`}
        />
      )}
      <div className="flex items-center gap-3">
        {icon}
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">{subtitle}</div>
          {meta}
        </div>
        {trailing}
      </div>
      {children}
    </div>
  )
}

/**
 * A section below the header, divided from it by a hairline.
 *
 * Every card that previews its own content — a script's first real line, an
 * approval's message, a loop's body — drew the same rule and the same spacing
 * by hand. `rows` is the one real variation: a list styles its own lines, so it
 * takes the divider and the spacing without the single-line preview treatment.
 */
export function NodeFooter({
  mono,
  rows,
  children
}: {
  mono?: boolean
  rows?: boolean
  children: ReactNode
}) {
  const inner = rows
    ? 'space-y-0.5'
    : `text-[11px] text-gray-600 truncate ${mono ? 'font-mono' : ''}`
  return <div className={`mt-2 border-t border-white/[0.06] pt-2 ${inner}`}>{children}</div>
}
