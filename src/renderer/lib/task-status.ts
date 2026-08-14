import { TaskStatus } from '../../shared/types'
import { Circle, Clock, Eye, CheckCircle2, XCircle } from 'lucide-react'
import { TONE_DOT, TONE_DOT_MOVING, TONE_TEXT, type StatusTone } from './status-tone'

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

export const TASK_STATUS_TEXT: Record<TaskStatus, string> = {
  todo: TONE_TEXT[TASK_STATUS_TONE.todo],
  in_progress: TONE_TEXT[TASK_STATUS_TONE.in_progress],
  in_review: TONE_TEXT[TASK_STATUS_TONE.in_review],
  done: TONE_TEXT[TASK_STATUS_TONE.done],
  cancelled: TONE_TEXT[TASK_STATUS_TONE.cancelled]
}

export const TASK_STATUS_DOT: Record<TaskStatus, string> = {
  todo: TONE_DOT[TASK_STATUS_TONE.todo],
  in_progress: TONE_DOT[TASK_STATUS_TONE.in_progress],
  in_review: TONE_DOT[TASK_STATUS_TONE.in_review],
  done: TONE_DOT[TASK_STATUS_TONE.done],
  cancelled: TONE_DOT[TASK_STATUS_TONE.cancelled]
}

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
