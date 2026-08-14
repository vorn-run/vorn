import { TaskStatus } from '../../shared/types'
import { Circle, Clock, Eye, CheckCircle2, XCircle } from 'lucide-react'
import { byTone, TONE_DOT, TONE_DOT_MOVING, TONE_TEXT, type StatusTone } from './status-tone'

/**
 * What each task state means, in the shared vocabulary.
 *
 * `in_review` is the accent, and it marks the same relationship the accent marks
 * everywhere else: an agent has finished and is waiting on the person. A waiting
 * session, an open approval gate and a task handed back for review are one idea
 * wearing three names.
 *
 * This replaces five maps that disagreed. `in_progress` alone had six values —
 * blue in four of them, and `text-yellow-500` in the one map that actually
 * reached a card, so the icon on every in-progress task was yellow while three
 * unread maps insisted it was blue. `StatusPicker` drew the icon from one map and
 * the word beside it from another, on the same row.
 */
export const TASK_STATUS_TONE: Record<TaskStatus, StatusTone> = {
  todo: 'idle',
  in_progress: 'live',
  in_review: 'blocked',
  done: 'settled',
  cancelled: 'idle'
}

/**
 * The order a person reads the statuses in, and the definition of "all of
 * them".
 *
 * The kanban columns, the list sections, the toolbar filter and the status
 * picker each carried their own copy of this array, so adding or reordering a
 * status meant four edits and any one of them could present a different set.
 */
export const TASK_STATUS_ORDER: TaskStatus[] = [
  'todo',
  'in_progress',
  'in_review',
  'done',
  'cancelled'
]

/**
 * The label for each status, in one place.
 *
 * These were written out four times — the kanban columns, the list sections, the
 * toolbar filter and the status picker — which is how the toolbar ended up
 * painting "All" in the colour it used for `todo`.
 */
export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  todo: 'Todo',
  in_progress: 'In Progress',
  in_review: 'In Review',
  done: 'Done',
  cancelled: 'Cancelled'
}

/**
 * A task carries its status as a glyph rather than a dot — the one surface where
 * the shape does the work and the colour only reinforces it.
 */
export const TASK_STATUS_ICON: Record<
  TaskStatus,
  React.FC<{ size?: number; className?: string }>
> = {
  todo: Circle,
  in_progress: Clock,
  in_review: Eye,
  done: CheckCircle2,
  cancelled: XCircle
}

/**
 * Two axes, not one.
 *
 * `TASK_STATUS_TONE` above says how urgent a state is, and that is what the rest
 * of the app reads — it is why `in_review` takes the accent. But a task board is
 * also the only surface *organised* by status: a long list sorted into columns,
 * read by scanning for one. A column of identical grey rows is hard to scan and,
 * frankly, dispiriting to look at.
 *
 * So status here also carries a category colour, kept at low saturation so the
 * board reads as a set. `in_review` defers to the accent rather than taking a
 * category colour of its own, which is what keeps bronzo the brightest thing on
 * the board and stops it becoming just another column tint. `cancelled` keeps no
 * colour at all: an abandoned task is an absence, not a category.
 */
const CATEGORY: Partial<Record<TaskStatus, { text: string; dot: string }>> = {
  todo: { text: 'text-status-slate', dot: 'bg-status-slate' },
  in_progress: { text: 'text-status-blue', dot: 'bg-status-blue' },
  done: { text: 'text-status-sage', dot: 'bg-status-sage' }
}

/**
 * Tone first, then a category colour where the board has one. Written as one
 * function over both axes so the word and the dot for a status cannot be given
 * different answers — they used to be two hand-listed maps kept in step by a
 * test that compared them with string surgery.
 */
function categorised(table: Record<StatusTone, string>, axis: 'text' | 'dot') {
  const out = byTone(TASK_STATUS_TONE, table)
  for (const status of TASK_STATUS_ORDER) {
    const pair = CATEGORY[status]
    if (pair) out[status] = pair[axis]
  }
  return out
}

export const TASK_STATUS_TEXT = categorised(TONE_TEXT, 'text')
export const TASK_STATUS_DOT = categorised(TONE_DOT, 'dot')

/** The live-session dot on a card — the one moving thing on this surface. */
export const TASK_LIVE_DOT = TONE_DOT_MOVING.live

export function formatTaskDate(dateStr: string): string {
  const d = new Date(dateStr)
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

export function getTaskShortId(task: { projectName: string; id: string }): string {
  const prefix =
    task.projectName
      .replace(/[^a-zA-Z]/g, '')
      .slice(0, 3)
      .toUpperCase() || 'TSK'
  const suffix = task.id.slice(0, 4).toUpperCase()
  return `${prefix}-${suffix}`
}
