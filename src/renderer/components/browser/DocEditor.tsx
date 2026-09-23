import { useMemo } from 'react'
import { RichMarkdownEditor } from '../rich-editor/RichMarkdownEditor'
import { mergeDocEdit } from '../../lib/doc-edits'

/** The bar while a doc is being edited: which version it starts from, and the way out. */
export function DocEditBar({
  version,
  onDiscard
}: {
  version: number
  onDiscard: () => void
}): React.JSX.Element {
  return (
    <>
      <span
        className="flex items-center h-6 px-2 rounded border border-white/[0.08] font-mono
                   text-[11px] text-ink-secondary shrink-0"
      >
        v{version}
      </span>
      <span className="ml-1.5 font-mono text-[11px] text-ink-faint truncate min-w-0">
        doc · you are editing
      </span>
      <span className="flex-1" />
      <button
        type="button"
        onClick={onDiscard}
        className="h-6 px-2.5 rounded text-[12px] text-ink-secondary hover:bg-white/[0.06] shrink-0"
      >
        Discard
      </button>
    </>
  )
}

interface Props {
  original: string
  edited: string
  onChange: (md: string) => void
  /** Comments already waiting to go, which a Save and send takes along. */
  drafts: number
  next: number
  agent: string
  saving: boolean
  onSave: (send: boolean) => void
}

/** A doc open for editing, with the edits it will send listed beside it as before and after. */
export function DocEditor({
  original,
  edited,
  onChange,
  drafts,
  next,
  agent,
  saving,
  onSave
}: Props): React.JSX.Element {
  const { edits } = useMemo(() => mergeDocEdit(original, edited), [original, edited])
  const total = edits.length + drafts
  return (
    <div className="absolute inset-0 z-10 flex min-h-0 bg-surface-base">
      <div className="flex-1 min-w-0 overflow-y-auto p-4">
        <RichMarkdownEditor value={edited} onChange={onChange} placeholder="" />
      </div>
      <aside
        aria-label="Your edits"
        className="w-[260px] shrink-0 flex flex-col min-h-0 border-l border-white/[0.06] bg-surface-panel"
      >
        <div className="flex-1 min-h-0 overflow-y-auto">
          <h4
            className="flex justify-between px-3 py-2 border-b border-white/[0.04] font-mono
                       text-[11px] font-semibold tracking-wider uppercase text-ink-faint"
          >
            <span>To send</span>
            <span>{total}</span>
          </h4>
          {edits.length === 0 ? (
            <p className="px-3 py-3 text-[12px] text-ink-faint">
              Change the words; each changed paragraph goes as its own edit.
            </p>
          ) : (
            <ul>
              {edits.map((e, i) => (
                <li
                  key={i}
                  className="flex flex-col gap-1 px-3 py-2.5 border-b border-white/[0.04] text-[12px]"
                >
                  <span className="font-mono text-[11px] text-ink-faint">Your edit</span>
                  {e.before && (
                    <p className="text-ink-faint line-through line-clamp-3 break-words">
                      {e.before}
                    </p>
                  )}
                  {e.after && <p className="text-ink line-clamp-3 break-words">{e.after}</p>}
                </li>
              ))}
            </ul>
          )}
          {drafts > 0 && (
            <p className="px-3 py-2.5 text-[12px] text-ink-faint">
              And {drafts} comment{drafts === 1 ? '' : 's'} already waiting.
            </p>
          )}
        </div>
        <div className="flex flex-col gap-2 p-3 border-t border-white/[0.06]">
          <button
            type="button"
            onClick={() => onSave(true)}
            disabled={edits.length === 0 || saving}
            className="h-7 rounded bg-ink text-surface-base text-[12px] font-semibold disabled:opacity-40"
          >
            Save v{next} and send {total} to {agent}
          </button>
          <button
            type="button"
            onClick={() => onSave(false)}
            disabled={edits.length === 0 || saving}
            className="h-6 rounded text-[12px] text-ink-secondary hover:bg-white/[0.06] disabled:opacity-40"
          >
            Save v{next} without sending
          </button>
        </div>
      </aside>
    </div>
  )
}
