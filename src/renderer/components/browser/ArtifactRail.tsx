import { useState } from 'react'
import type { ArtifactComment } from '../../../shared/types'

interface Props {
  drafts: ArtifactComment[]
  /** The last batch sent, shown with whether its words survived into this version. */
  sent: ArtifactComment[]
  version: number
  /** Which anchors found their words on the page in front. */
  found: Record<string, boolean>
  agent: string
  queued: boolean
  sending: boolean
  onSend: () => void
  onEdit: (id: string, body: string) => void
  onDelete: (id: string) => void
  onReveal: (comment: ArtifactComment) => void
  onAddNote: (body: string) => void
}

/** Where a comment points, as the rail quotes it. */
function Target({ comment }: { comment: ArtifactComment }): React.JSX.Element {
  const a = comment.anchor
  const text =
    a?.kind === 'quote'
      ? a.quote
      : a?.kind === 'edit'
        ? `${a.before} → ${a.after}`
        : a?.kind === 'point'
          ? `${a.artboard} · ${a.element}`
          : 'The whole version'
  return (
    <q
      className="block font-mono text-[11.5px] leading-snug text-ink-faint border-l-2
                 border-status-blue pl-2 line-clamp-2 before:content-none after:content-none"
    >
      {text}
    </q>
  )
}

function Draft({
  comment,
  missing,
  onEdit,
  onDelete,
  onReveal
}: {
  comment: ArtifactComment
  missing: boolean
  onEdit: (body: string) => void
  onDelete: () => void
  onReveal: () => void
}): React.JSX.Element {
  const [editing, setEditing] = useState<string | null>(null)
  return (
    <li className="flex flex-col gap-1.5 px-3 py-2.5 border-b border-white/[0.04]">
      <button type="button" onClick={onReveal} className="text-left" aria-label="Show on the page">
        <Target comment={comment} />
      </button>
      {editing === null ? (
        <p className="text-[12px] text-ink whitespace-pre-wrap break-words">{comment.body}</p>
      ) : (
        <textarea
          autoFocus
          value={editing}
          onChange={(e) => setEditing(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation()
            if (e.key === 'Escape') setEditing(null)
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && editing.trim()) {
              onEdit(editing.trim())
              setEditing(null)
            }
          }}
          aria-label="Edit comment"
          rows={3}
          className="w-full resize-none rounded border border-white/[0.12] bg-surface-sunken
                     px-2 py-1.5 text-[12px] text-ink outline-none focus:border-white/[0.24]"
        />
      )}
      <div className="flex items-center gap-2 font-mono text-[11px] text-ink-faint">
        {missing && <span className="text-danger">words changed</span>}
        {editing === null ? (
          <>
            <button
              type="button"
              onClick={() => setEditing(comment.body)}
              className="hover:text-ink"
            >
              edit
            </button>
            <button type="button" onClick={onDelete} className="hover:text-danger">
              delete
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => {
                if (editing.trim()) onEdit(editing.trim())
                setEditing(null)
              }}
              className="hover:text-ink"
            >
              save
            </button>
            <button type="button" onClick={() => setEditing(null)} className="hover:text-ink">
              cancel
            </button>
          </>
        )}
      </div>
    </li>
  )
}

/** The batch being gathered for the agent, and what became of the last one. */
export function ArtifactRail({
  drafts,
  sent,
  version,
  found,
  agent,
  queued,
  sending,
  onSend,
  onEdit,
  onDelete,
  onReveal,
  onAddNote
}: Props): React.JSX.Element {
  const [note, setNote] = useState('')
  const sentOn = sent.length ? Math.max(...sent.map((c) => c.version)) : 0
  const addNote = (): void => {
    if (!note.trim()) return
    onAddNote(note.trim())
    setNote('')
  }
  return (
    <aside
      aria-label="Comments"
      className="w-[260px] shrink-0 flex flex-col min-h-0 border-l border-white/[0.06] bg-surface-panel"
    >
      <div className="flex-1 min-h-0 overflow-y-auto">
        <h4
          className="flex justify-between px-3 py-2 border-b border-white/[0.04] font-mono
                     text-[11px] font-semibold tracking-wider uppercase text-ink-faint"
        >
          <span>To send</span>
          <span>{drafts.length}</span>
        </h4>
        {drafts.length === 0 ? (
          <p className="px-3 py-3 text-[12px] text-ink-faint">
            Select words on the page to comment on them.
          </p>
        ) : (
          <ul>
            {drafts.map((c) => (
              <Draft
                key={c.id}
                comment={c}
                missing={c.anchor?.kind === 'quote' && found[c.id] === false}
                onEdit={(body) => onEdit(c.id, body)}
                onDelete={() => onDelete(c.id)}
                onReveal={() => onReveal(c)}
              />
            ))}
          </ul>
        )}
        {sent.length > 0 && (
          <>
            <h4
              className="px-3 py-2 border-b border-white/[0.04] font-mono text-[11px]
                         font-semibold tracking-wider uppercase text-ink-faint"
            >
              Sent with v{sentOn}
            </h4>
            <ul>
              {sent.map((c) => {
                const moved = version > c.version
                const lost = c.anchor?.kind === 'quote' && found[c.id] === false
                return (
                  <li
                    key={c.id}
                    className="flex flex-col gap-1.5 px-3 py-2.5 border-b border-white/[0.04]"
                  >
                    <button
                      type="button"
                      onClick={() => onReveal(c)}
                      className="text-left"
                      aria-label="Show on the page"
                    >
                      <Target comment={c} />
                    </button>
                    <p className="text-[12px] text-ink-secondary whitespace-pre-wrap break-words">
                      {c.body}
                    </p>
                    <span
                      className={`font-mono text-[11px] ${lost && moved ? 'text-danger' : 'text-ink-faint'}`}
                    >
                      {!moved ? 'sent' : lost ? `words changed in v${version}` : 'still anchored'}
                    </span>
                  </li>
                )
              })}
            </ul>
          </>
        )}
      </div>
      <div className="flex flex-col gap-2 p-3 border-t border-white/[0.06]">
        <textarea
          value={note}
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation()
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) addNote()
          }}
          aria-label="Note about the whole version"
          placeholder="A note about the whole version"
          rows={2}
          className="w-full resize-none rounded border border-white/[0.12] bg-surface-sunken
                     px-2 py-1.5 text-[12px] text-ink outline-none focus:border-white/[0.24]
                     placeholder:text-ink-faint"
        />
        {note.trim() && (
          <button
            type="button"
            onClick={addNote}
            className="h-6 rounded border border-white/[0.12] text-[12px] text-ink hover:bg-white/[0.06]"
          >
            Add note
          </button>
        )}
        <button
          type="button"
          onClick={onSend}
          disabled={drafts.length === 0 || sending}
          className="h-7 rounded bg-ink text-surface-base text-[12px] font-semibold disabled:opacity-40"
        >
          {drafts.length
            ? `Send ${drafts.length} comment${drafts.length === 1 ? '' : 's'} to ${agent}`
            : 'Nothing to send yet'}
        </button>
        {queued && (
          <p className="text-[11.5px] text-ink-faint">
            Queued. It goes once {agent} is back at its prompt.
          </p>
        )}
      </div>
    </aside>
  )
}
