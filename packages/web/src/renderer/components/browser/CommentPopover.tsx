import { useState } from 'react'
import { POPOVER_WIDTH as WIDTH } from '../../lib/artifact-comments'

interface Props {
  quote: string
  /** Where the selection sits, in the page area's own coordinates. */
  at: { x: number; y: number }
  onAdd: (body: string) => void
  onCancel: () => void
}

/** A note on the words just selected, opened beside them. */
export function CommentPopover({ quote, at, onAdd, onCancel }: Props): React.JSX.Element {
  const [body, setBody] = useState('')
  const add = (): void => {
    if (body.trim()) onAdd(body.trim())
  }
  return (
    <div
      role="dialog"
      aria-label="Comment on the selection"
      className="absolute z-20 flex flex-col gap-2 p-2.5 rounded border border-white/[0.12]
                 text-[12px] text-ink"
      style={{ left: at.x, top: at.y, width: WIDTH, background: 'var(--color-surface-overlay)' }}
    >
      <q
        className="block font-mono text-[11.5px] leading-snug text-ink-faint border-l-2
                   border-status-blue pl-2 line-clamp-3 before:content-none after:content-none"
      >
        {quote}
      </q>
      <textarea
        autoFocus
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation()
          if (e.key === 'Escape') onCancel()
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) add()
        }}
        aria-label="Comment"
        placeholder="What should change here?"
        rows={3}
        className="w-full resize-none rounded border border-white/[0.12] bg-surface-sunken
                   px-2 py-1.5 text-[12px] text-ink outline-none focus:border-white/[0.24]
                   placeholder:text-ink-faint"
      />
      <div className="flex items-center gap-1.5">
        <span className="flex-1 font-mono text-[10.5px] text-ink-faint">⌘↵ to add</span>
        <button
          type="button"
          onClick={onCancel}
          className="h-6 px-2 rounded text-ink-secondary hover:bg-white/[0.06]"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={add}
          disabled={!body.trim()}
          className="h-6 px-2.5 rounded bg-ink text-surface-base font-semibold disabled:opacity-40"
        >
          Add
        </button>
      </div>
    </div>
  )
}
