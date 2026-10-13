import { ArrowLeft, ArrowRight, RotateCw, X } from 'lucide-react'

interface Props {
  draft: string
  onDraftChange: (value: string) => void
  onSubmit: () => void
  /** Escape puts the address back to the page actually loaded. */
  onRevert: () => void
  onBack: () => void
  onForward: () => void
  onReloadOrStop: () => void
  canGoBack: boolean
  canGoForward: boolean
  loading: boolean
  /** The shared icon-button class, so this bar matches the pane it sits in. */
  btn: string
}

/**
 * The pane's address bar: history, reload, and somewhere to type a url.
 *
 * Extracted so the header's one real decision — address bar, or the controls a
 * design declared — reads as a single line at the call site rather than as a
 * branch wrapped around fifty lines of unchanged markup.
 */
export function AddressBar({
  draft,
  onDraftChange,
  onSubmit,
  onRevert,
  onBack,
  onForward,
  onReloadOrStop,
  canGoBack,
  canGoForward,
  loading,
  btn
}: Props): React.JSX.Element {
  return (
    <>
      <button onClick={onBack} disabled={!canGoBack} aria-label="Go back" className={btn}>
        <ArrowLeft size={14} strokeWidth={2} />
      </button>
      <button onClick={onForward} disabled={!canGoForward} aria-label="Go forward" className={btn}>
        <ArrowRight size={14} strokeWidth={2} />
      </button>
      <button
        onClick={onReloadOrStop}
        aria-label={loading ? 'Stop loading' : 'Reload'}
        className={btn}
      >
        {loading ? <X size={14} strokeWidth={2} /> : <RotateCw size={14} strokeWidth={2} />}
      </button>

      <form
        className="flex-1 min-w-0 ml-1"
        onSubmit={(e) => {
          e.preventDefault()
          onSubmit()
        }}
      >
        <input
          value={draft}
          onChange={(e) => onDraftChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') onRevert()
            e.stopPropagation()
          }}
          spellCheck={false}
          aria-label="Address"
          placeholder="Type a URL"
          className="w-full bg-white/[0.04] hover:bg-white/[0.06] focus:bg-white/[0.07]
                         rounded-full px-3 py-1 text-[11px] text-gray-300 outline-none
                         text-center focus:text-left focus:font-mono placeholder:text-gray-500
                         transition-colors"
        />
      </form>
    </>
  )
}
