import { memo, forwardRef, useState, useRef, useEffect, useCallback } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { ArrowLeft, ArrowRight, Globe, RotateCw, X } from 'lucide-react'
import { useAppStore } from '../stores'
import { PaneCard } from './PaneCard'
import { browserPaneId } from '../lib/pane-id'
import { normalizeUrl, displayHost } from '../lib/browser-url'

interface Props {
  /** Session that owns this browser. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * Minimal `<webview>` element typing.
 *
 * Electron's webview is a custom element, so React has no JSX types for it and
 * its imperative API isn't part of the DOM lib. Only the parts used here are
 * declared rather than pulling in a broader shim.
 */
interface WebviewElement extends HTMLElement {
  src: string
  canGoBack(): boolean
  canGoForward(): boolean
  goBack(): void
  goForward(): void
  reload(): void
  stop(): void
  getURL(): string
}

/**
 * A session's browser, as its own grid pane.
 *
 * One per session, like its tree and its editor — so a session can keep its dev
 * server or a doc page beside the agent working on it. The page runs in a
 * separate process; `hardenWebviews` in the main process strips its privileges.
 */
export const BrowserCard = memo(
  forwardRef<HTMLDivElement, Props>(function BrowserCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, url, openBrowserPane, closeBrowserPane } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        url: s.browserPanes.get(sessionId)?.url ?? null,
        openBrowserPane: s.openBrowserPane,
        closeBrowserPane: s.closeBrowserPane
      }))
    )

    const viewRef = useRef<WebviewElement | null>(null)
    const [draft, setDraft] = useState(url ?? '')
    const [loading, setLoading] = useState(false)
    const [failed, setFailed] = useState<string | null>(null)
    const [nav, setNav] = useState({ back: false, forward: false })

    // Follow store-driven navigation (the address bar writes through the store,
    // so this also covers in-page redirects reported back by the guest).
    useEffect(() => {
      setDraft(url ?? '')
    }, [url])

    useEffect(() => {
      const view = viewRef.current
      if (!view) return

      const syncNav = (): void => {
        setNav({ back: view.canGoBack(), forward: view.canGoForward() })
      }
      const onStart = (): void => {
        setLoading(true)
        setFailed(null)
      }
      const onStop = (): void => {
        setLoading(false)
        syncNav()
      }
      const onFail = (e: Event): void => {
        // -3 is ERR_ABORTED, which fires for ordinary navigation cancellation.
        const detail = e as Event & { errorCode?: number; errorDescription?: string }
        if (detail.errorCode === -3) return
        setLoading(false)
        setFailed(detail.errorDescription || 'Failed to load')
      }

      view.addEventListener('did-start-loading', onStart)
      view.addEventListener('did-stop-loading', onStop)
      view.addEventListener('did-fail-load', onFail)
      return () => {
        view.removeEventListener('did-start-loading', onStart)
        view.removeEventListener('did-stop-loading', onStop)
        view.removeEventListener('did-fail-load', onFail)
      }
    }, [])

    const commitUrl = useCallback(
      (raw: string) => {
        const normalized = normalizeUrl(raw)
        if (!normalized) {
          setFailed('That does not look like a web address')
          return
        }
        setFailed(null)
        openBrowserPane(sessionId, normalized)
      },
      [openBrowserPane, sessionId]
    )

    if (!terminal || url === null) return null

    return (
      <PaneCard
        ref={ref}
        paneId={browserPaneId(sessionId)}
        title={displayHost(url)}
        icon={<Globe size={12} className="text-gray-500 shrink-0" />}
        onClose={() => closeBrowserPane(sessionId)}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
      >
        {/* Address bar */}
        <div
          className="flex items-center gap-1 px-2 py-1 border-b border-white/[0.06] shrink-0"
          style={{ background: '#1e1e22' }}
        >
          <button
            onClick={() => viewRef.current?.goBack()}
            disabled={!nav.back}
            aria-label="Go back"
            className="p-0.5 rounded text-gray-500 enabled:hover:text-white disabled:opacity-30 transition-colors"
          >
            <ArrowLeft size={12} strokeWidth={2} />
          </button>
          <button
            onClick={() => viewRef.current?.goForward()}
            disabled={!nav.forward}
            aria-label="Go forward"
            className="p-0.5 rounded text-gray-500 enabled:hover:text-white disabled:opacity-30 transition-colors"
          >
            <ArrowRight size={12} strokeWidth={2} />
          </button>
          <button
            onClick={() => (loading ? viewRef.current?.stop() : viewRef.current?.reload())}
            aria-label={loading ? 'Stop loading' : 'Reload'}
            className="p-0.5 rounded text-gray-500 hover:text-white transition-colors"
          >
            {loading ? <X size={12} strokeWidth={2} /> : <RotateCw size={12} strokeWidth={2} />}
          </button>

          <form
            className="flex-1 min-w-0"
            onSubmit={(e) => {
              e.preventDefault()
              commitUrl(draft)
            }}
          >
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setDraft(url)
                e.stopPropagation()
              }}
              spellCheck={false}
              aria-label="Address"
              placeholder="localhost:5173"
              className="w-full bg-white/[0.04] focus:bg-white/[0.06] rounded px-2 py-0.5
                         text-[11px] font-mono text-gray-200 outline-none
                         placeholder:text-gray-600"
            />
          </form>
        </div>

        {failed && (
          <div className="px-2 py-1 text-[10px] text-amber-400/90 border-b border-white/[0.06] shrink-0">
            {failed}
          </div>
        )}

        <div className="flex-1 min-h-0 relative" style={{ background: '#fff' }}>
          <webview
            ref={viewRef as unknown as React.Ref<HTMLElement>}
            src={url}
            // Each session browses in its own partition, so logins and cookies
            // in one session's pane don't leak into another's.
            partition={`persist:vorn-browser-${sessionId}`}
            className="absolute inset-0 w-full h-full"
          />
        </div>
      </PaneCard>
    )
  })
)
