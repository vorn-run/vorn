import { TaskConfig, isTerminalTaskStatus } from '../../../shared/types'
import { AgentIcon } from '../AgentIcon'
import { SourceBadge } from '../ConnectorIcon'
import { useConnectorGlyph } from '../../lib/use-connections'
import {
  TASK_STATUS_ICON,
  TASK_STATUS_TEXT,
  formatTaskDate,
  getTaskShortId,
  TASK_LIVE_DOT
} from '../../lib/task-status'
import {
  Pencil,
  Trash2,
  Play,
  Terminal,
  RotateCcw,
  FileCode,
  XCircle,
  CheckCircle2,
  Archive,
  ArchiveRestore
} from 'lucide-react'
import { ConfirmPopover } from '../ConfirmPopover'
import { KanbanCardMenu } from './KanbanCardMenu'

export function TaskCard({
  task,
  onEdit,
  onDelete,
  onOpenSession,
  onComplete,
  onCancel,
  onReopen,
  onArchive,
  onUnarchive,
  onReviewDiff,
  onSelect,
  sessionIsLive,
  variant = 'default'
}: {
  task: TaskConfig
  onEdit: () => void
  onDelete: () => void
  onOpenSession?: () => void
  onComplete?: () => void
  onCancel?: () => void
  onReopen?: () => void
  onArchive?: () => void
  onUnarchive?: () => void
  onReviewDiff?: () => void
  onSelect?: () => void
  sessionIsLive?: boolean
  variant?: 'default' | 'kanban'
}) {
  const StatusIcon = TASK_STATUS_ICON[task.status]
  const iconColor = TASK_STATUS_TEXT[task.status]
  const shortId = getTaskShortId(task)
  const sourceGlyph = useConnectorGlyph(task.sourceConnectorId)
  const isArchived = !!task.archivedAt
  const dimmed = task.status === 'cancelled' || isArchived

  if (variant === 'kanban') {
    return (
      // The same relationship a workflow node has to its canvas: an opaque rung of
      // the ladder plus a hairline, not a translucent wash. A wash over the field
      // greys toward the white it is made of, which is why these cards read pale
      // while nodes at the same lightness read as part of the app.
      <div
        className={`group relative bg-surface-raised border border-white/[0.08] rounded-md overflow-hidden
                     hover:border-white/[0.16]
                     transition-colors duration-150
                     ${dimmed ? 'opacity-60' : ''} ${onSelect ? 'cursor-pointer' : ''}`}
        onClick={onSelect}
      >
        <div className="px-3.5 py-3">
          {/* Top row: short ID + source badge + menu */}
          <div className="flex items-center justify-between mb-1.5">
            <span className="flex items-center gap-1.5">
              <span className="text-[10px] text-ink-faint font-medium">{shortId}</span>
              {task.sourceConnectorId && task.sourceExternalId && (
                <>
                  <span className="text-[10px] text-ink-faint">·</span>
                  <SourceBadge
                    connectorId={task.sourceConnectorId}
                    icon={sourceGlyph}
                    url={task.sourceExternalUrl}
                    label={`#${task.sourceExternalId}`}
                  />
                </>
              )}
              {isArchived && (
                <Archive
                  size={10}
                  strokeWidth={2}
                  className="text-ink-faint"
                  aria-label="Archived"
                />
              )}
            </span>
            <div
              className="opacity-0 group-hover:opacity-100 transition-opacity"
              onClick={(e) => e.stopPropagation()}
            >
              <KanbanCardMenu
                status={task.status}
                isArchived={isArchived}
                onEdit={onEdit}
                onDelete={onDelete}
                onOpenSession={onOpenSession}
                onComplete={onComplete}
                onCancel={onCancel}
                onReopen={onReopen}
                onArchive={onArchive}
                onUnarchive={onUnarchive}
                onReviewDiff={onReviewDiff}
                sessionIsLive={sessionIsLive}
              />
            </div>
          </div>

          {/* Status icon + Title */}
          <div className="flex items-start gap-1.5">
            <StatusIcon size={14} className={`${iconColor} mt-0.5 shrink-0`} />
            <span
              className={`text-[13px] font-medium leading-snug line-clamp-2
                           ${task.status === 'cancelled' ? 'text-ink-faint line-through' : 'text-ink'}`}
            >
              {task.title}
            </span>
          </div>

          {/* Footer: agent/session on left, date on right */}
          <div className="flex items-center justify-between mt-2.5">
            <div className="flex items-center gap-1.5">
              {task.assignedAgent && (
                <span className="flex items-center gap-1">
                  <AgentIcon agentType={task.assignedAgent} size={13} />
                  {sessionIsLive && (
                    <span className={`w-1.5 h-1.5 rounded-full ${TASK_LIVE_DOT}`} />
                  )}
                </span>
              )}
            </div>
            <span className="text-[11px] text-ink-faint">
              Created {formatTaskDate(task.createdAt)}
            </span>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className={`group flex items-center gap-3 px-3 py-2 hover:bg-white/[0.04] rounded-md transition-colors
                   ${dimmed ? 'opacity-60' : ''} ${onSelect ? 'cursor-pointer' : ''}`}
      onClick={onSelect}
    >
      {/* Agent icon or priority placeholder */}
      <div className="w-4 shrink-0 flex items-center justify-center">
        {task.assignedAgent ? (
          <AgentIcon agentType={task.assignedAgent} size={14} />
        ) : (
          <span className="text-[10px] text-ink-faint">---</span>
        )}
      </div>

      {/* Task short ID + source badge */}
      <span className="flex items-center gap-1.5 text-[11px] text-ink-faint font-medium shrink-0">
        {shortId}
        {task.sourceConnectorId && task.sourceExternalId && (
          <SourceBadge
            connectorId={task.sourceConnectorId}
            icon={sourceGlyph}
            url={task.sourceExternalUrl}
            label={`#${task.sourceExternalId}`}
          />
        )}
        {isArchived && (
          <Archive size={10} strokeWidth={2} className="text-ink-faint" aria-label="Archived" />
        )}
      </span>

      {/* Status icon */}
      <StatusIcon size={14} className={`${iconColor} shrink-0`} />

      {/* Title */}
      <span
        className={`text-sm truncate flex-1 min-w-0 ${
          task.status === 'cancelled' ? 'text-ink-faint line-through' : 'text-ink'
        }`}
      >
        {task.title}
      </span>

      {/* Right side: date */}
      <span className="text-[11px] text-ink-faint shrink-0">{formatTaskDate(task.createdAt)}</span>

      {/* Hover actions */}
      <div
        className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
        onClick={(e) => e.stopPropagation()}
      >
        {task.status === 'in_review' && onReviewDiff && (
          <button
            onClick={onReviewDiff}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Review diff"
          >
            <FileCode size={12} strokeWidth={2} />
          </button>
        )}
        {task.status === 'in_review' && onComplete && (
          <button
            onClick={onComplete}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Mark as done"
          >
            <CheckCircle2 size={12} strokeWidth={2} />
          </button>
        )}
        {task.status === 'cancelled' && onReopen && (
          <button
            onClick={onReopen}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Reopen task"
          >
            <RotateCcw size={12} strokeWidth={2} />
          </button>
        )}
        {onOpenSession && (
          <button
            onClick={onOpenSession}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title={sessionIsLive ? 'Focus session' : 'Resume session'}
          >
            {sessionIsLive ? (
              <Terminal size={12} strokeWidth={2} />
            ) : (
              <Play size={12} strokeWidth={2} />
            )}
          </button>
        )}
        <button
          onClick={onEdit}
          className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
          title="Edit task"
        >
          <Pencil size={12} strokeWidth={2} />
        </button>
        {task.status !== 'cancelled' && task.status !== 'done' && onCancel && (
          <button
            onClick={onCancel}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Cancel task"
          >
            <XCircle size={12} strokeWidth={2} />
          </button>
        )}
        {!isArchived && isTerminalTaskStatus(task.status) && onArchive && (
          <button
            onClick={onArchive}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Archive task"
          >
            <Archive size={12} strokeWidth={2} />
          </button>
        )}
        {isArchived && onUnarchive && (
          <button
            onClick={onUnarchive}
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Unarchive task"
          >
            <ArchiveRestore size={12} strokeWidth={2} />
          </button>
        )}
        <ConfirmPopover
          message="Delete this task permanently?"
          confirmLabel="Delete"
          onConfirm={onDelete}
        >
          <button
            className="p-1 text-ink-faint hover:text-ink rounded-sm transition-colors"
            title="Delete task"
          >
            <Trash2 size={12} strokeWidth={2} />
          </button>
        </ConfirmPopover>
      </div>
    </div>
  )
}
