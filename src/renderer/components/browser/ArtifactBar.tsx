import { createPortal } from 'react-dom'
import { ChevronDown, MessageSquarePlus } from 'lucide-react'
import type { ArtifactComment, ArtifactVersion } from '../../../shared/types'
import { useAnchoredMenu } from '../../hooks/useAnchoredMenu'
import { formatRelativeTime } from '../../lib/format-time'
import { Tooltip } from '../Tooltip'

const MENU_ROW_PX = 26

interface Props {
  version: number
  versions: ArtifactVersion[]
  comments: ArtifactComment[]
  /** Who a Send goes to, as the session names its agent. */
  agent: string
  commenting: boolean
  /** Absent where comments cannot be read off the page, as on a popped-out card. */
  onToggleComments?: () => void
  onSelectVersion: (version: number) => void
  onSend?: () => void
  /** Offered on a doc; null when it can be edited only from its latest version. */
  onEdit?: (() => void) | null
  sending: boolean
  queued: boolean
  btn: string
}

/** Who made a version and when, the way the bar and the menu both say it. */
function versionByline(v: ArtifactVersion | undefined, agent: string): string {
  if (!v) return ''
  return `${v.author === 'user' ? 'you' : agent} · ${formatRelativeTime(v.createdAt).toLowerCase()}`
}

/** An artifact tab's header: which version, who made it, and the way back to the agent. */
export function ArtifactBar({
  version,
  versions,
  comments,
  agent,
  commenting,
  onToggleComments,
  onSelectVersion,
  onSend,
  onEdit,
  sending,
  queued,
  btn
}: Props): React.JSX.Element {
  const { open, setOpen, toggle, triggerRef, menuRef, position } = useAnchoredMenu({
    estimateHeight: () => versions.length * MENU_ROW_PX + 12,
    menuWidth: () => 240
  })
  const drafts = comments.filter((c) => c.state === 'draft').length
  const current = versions.find((v) => v.version === version)
  const newest = [...versions].sort((a, b) => b.version - a.version)

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={toggle}
        aria-label={`Version ${version}, choose another`}
        aria-haspopup="menu"
        aria-expanded={open}
        className="flex items-center gap-1 h-6 px-2 rounded border border-white/[0.08]
                   hover:bg-white/[0.06] font-mono text-[11px] text-ink-secondary shrink-0"
      >
        v{version}
        <ChevronDown size={11} strokeWidth={2} />
      </button>
      <span className="ml-1.5 font-mono text-[11px] text-ink-faint truncate min-w-0">
        {versionByline(current, agent)}
      </span>
      <span className="flex-1" />

      {onEdit !== undefined && (
        <Tooltip
          label={onEdit ? 'Edit the words yourself' : 'Open the latest version to edit it'}
          position="bottom"
        >
          <button
            type="button"
            onClick={onEdit ?? undefined}
            disabled={!onEdit}
            className="mr-1 h-6 px-2.5 rounded border border-white/[0.12] text-[12px] text-ink
                       hover:bg-white/[0.06] disabled:opacity-40 disabled:hover:bg-transparent shrink-0"
          >
            Edit
          </button>
        </Tooltip>
      )}
      {onToggleComments && (
        <Tooltip label={commenting ? 'Stop commenting' : 'Comment on the page'} position="bottom">
          <button
            type="button"
            onClick={onToggleComments}
            aria-label={commenting ? 'Stop commenting' : 'Comment on the page'}
            aria-pressed={commenting}
            className={`${btn} ${commenting ? 'bg-white/[0.10]' : ''}`}
          >
            <MessageSquarePlus size={14} strokeWidth={2} />
          </button>
        </Tooltip>
      )}
      {onSend && (
        <Tooltip
          label={
            queued
              ? `Queued: goes when ${agent} is at its prompt`
              : drafts
                ? `Send ${drafts} to ${agent}`
                : 'Comment on the page first'
          }
          position="bottom"
        >
          <button
            type="button"
            onClick={onSend}
            disabled={drafts === 0 || sending}
            className="ml-1 flex items-center gap-1.5 h-6 px-2.5 rounded border border-white/[0.12]
                       text-[12px] text-ink hover:bg-white/[0.06] disabled:opacity-40
                       disabled:hover:bg-transparent shrink-0"
          >
            {queued ? 'Queued' : 'Send to agent'}
            <span
              className="grid place-items-center min-w-4 h-4 px-1 rounded-full bg-white/[0.12]
                         font-mono text-[10px] font-semibold"
            >
              {drafts}
            </span>
          </button>
        </Tooltip>
      )}

      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            aria-label="Versions"
            className="fixed z-[200] border border-white/[0.08] rounded p-1 overflow-y-auto"
            style={{
              background: 'var(--color-surface-overlay)',
              top: position.top,
              bottom: position.bottom,
              left: position.left,
              width: position.width,
              maxHeight: 'calc(100vh - 32px)'
            }}
          >
            {newest.map((v) => {
              const count = comments.filter((c) => c.version === v.version).length
              return (
                <button
                  key={v.version}
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    onSelectVersion(v.version)
                    setOpen(false)
                  }}
                  className={`w-full flex items-center gap-2 px-2 h-[26px] rounded text-left text-[12px] ${
                    v.version === version
                      ? 'bg-white/[0.08] text-ink'
                      : 'text-ink-secondary hover:bg-white/[0.05]'
                  }`}
                >
                  <span className="truncate flex-1">
                    v{v.version} · {v.author === 'user' ? 'you' : agent}
                    {count > 0 && ` · ${count} comment${count === 1 ? '' : 's'}`}
                  </span>
                  <span className="font-mono text-[11px] text-ink-faint shrink-0">
                    {formatRelativeTime(v.createdAt).toLowerCase()}
                  </span>
                </button>
              )
            })}
          </div>,
          document.body
        )}
    </>
  )
}
