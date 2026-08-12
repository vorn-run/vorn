import { memo, forwardRef, useState, useRef, useEffect, useCallback } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { ArrowLeft, ArrowRight, Plus, RotateCw, X } from 'lucide-react'
import { useAppStore } from '../stores'
import { PaneCard, PaneControls } from './PaneCard'
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
 * Electron's webview is a custom element, so its imperative API isn't part of
 * the DOM lib. Only the parts used here are declared rather than pulling in a
 * broader shim.
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
 *
 * Each tab keeps its own `<webview>` mounted and merely hidden while inactive,
 * so switching back doesn't reload the page or lose scroll position.
 */
export const BrowserCard = memo(
  forwardRef<HTMLDivElement, Props>(function BrowserCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const {
      terminal,
      pane,
      openBrowserPane,
      closeBrowserPane,
      addBrowserTab,
      closeBrowserTab,
      setActiveBrowserTab
    } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        pane: s.browserPanes.get(sessionId) ?? null,
        openBrowserPane: s.openBrowserPane,
        closeBrowserPane: s.closeBrowserPane,
        addBrowserTab: s.addBrowserTab,
        closeBrowserTab: s.closeBrowserTab,
        setActiveBrowserTab: s.setActiveBrowserTab
      }))
    )

    const url = pane ? (pane.tabs[pane.activeTab] ?? null) : null
    const viewRef = useRef<WebviewElement | null>(null)
    const [draft, setDraft] = useState(url ?? '')
    const [loading, setLoading] = useState(false)
    const [failed, setFailed] = useState<string | null>(null)
    const [nav, setNav] = useState({ back: false, forward: false })

    // Follow store-driven navigation, including a tab switch — the address bar
    // must show the page you are actually looking at. A blank tab shows its
    // placeholder instead of "about:blank", which is nothing you'd want to edit.
    useEffect(() => {
      setDraft(url === null || url === 'about:blank' ? '' : url)
      setFailed(null)
      setNav({ back: false, forward: false })
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
    }, [pane?.activeTab])

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

    if (!terminal || !pane || url === null) return null

    const btn =
      'p-1 rounded text-gray-500 enabled:hover:text-gray-200 enabled:hover:bg-white/[0.06] ' +
      'disabled:opacity-25 transition-colors'

    return (
      <PaneCard
        ref={ref}
        paneId={browserPaneId(sessionId)}
        title={displayHost(url)}
        onClose={() => closeBrowserPane(sessionId)}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // The tab strip is this pane's title bar; a second one above it would be
        // chrome stacked on chrome, and browsers don't have one.
        headerless
        background="#0d0d0f"
      >
        {/* Tab strip — doubles as the pane's header, so it carries the drag
            handle and the minimize / maximize / close cluster. */}
        <div
          className={`flex items-center gap-1 pl-1.5 pr-1 pt-1 shrink-0 ${
            onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''
          }`}
          onPointerDown={onDragStart ? (e) => onDragStart(browserPaneId(sessionId), e) : undefined}
        >
          <div
            className="flex items-stretch gap-0.5 flex-1 min-w-0 overflow-x-auto"
            role="tablist"
            aria-label="Browser tabs"
          >
            {pane.tabs.map((tabUrl, i) => {
              const active = i === pane.activeTab
              return (
                <div
                  key={i}
                  role="tab"
                  aria-selected={active}
                  tabIndex={0}
                  onClick={() => setActiveBrowserTab(sessionId, i)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') setActiveBrowserTab(sessionId, i)
                  }}
                  title={tabUrl}
                  className={`group/tab flex items-center gap-1 pl-2.5 pr-1 py-1 rounded-md max-w-[170px]
                              cursor-default select-none transition-colors ${
                                active
                                  ? 'bg-white/[0.06] text-gray-200'
                                  : 'text-gray-500 hover:text-gray-300 hover:bg-white/[0.03]'
                              }`}
                >
                  <span className="text-[11px] truncate">{displayHost(tabUrl)}</span>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      closeBrowserTab(sessionId, i)
                    }}
                    aria-label={`Close tab ${displayHost(tabUrl)}`}
                    className="shrink-0 p-0.5 rounded text-gray-600 hover:text-white
                               opacity-0 group-hover/tab:opacity-100 focus:opacity-100 transition-opacity"
                  >
                    <X size={10} strokeWidth={2.5} />
                  </button>
                </div>
              )
            })}
            <button
              type="button"
              onClick={() => addBrowserTab(sessionId)}
              aria-label="New tab"
              className="shrink-0 self-center ml-0.5 p-1 rounded-md text-gray-600 hover:text-gray-200
                         hover:bg-white/[0.06] transition-colors"
            >
              <Plus size={13} strokeWidth={2} />
            </button>
          </div>

          <PaneControls
            paneId={browserPaneId(sessionId)}
            title={displayHost(url)}
            onClose={() => closeBrowserPane(sessionId)}
            className="shrink-0"
          />
        </div>

        {/* Address bar */}
        <div className="flex items-center gap-0.5 px-1.5 py-1 shrink-0">
          <button
            onClick={() => viewRef.current?.goBack()}
            disabled={!nav.back}
            aria-label="Go back"
            className={btn}
          >
            <ArrowLeft size={14} strokeWidth={2} />
          </button>
          <button
            onClick={() => viewRef.current?.goForward()}
            disabled={!nav.forward}
            aria-label="Go forward"
            className={btn}
          >
            <ArrowRight size={14} strokeWidth={2} />
          </button>
          <button
            onClick={() => (loading ? viewRef.current?.stop() : viewRef.current?.reload())}
            aria-label={loading ? 'Stop loading' : 'Reload'}
            className={btn}
          >
            {loading ? <X size={14} strokeWidth={2} /> : <RotateCw size={14} strokeWidth={2} />}
          </button>

          <form
            className="flex-1 min-w-0 ml-1"
            onSubmit={(e) => {
              e.preventDefault()
              commitUrl(draft)
            }}
          >
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setDraft(url === 'about:blank' ? '' : url)
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
        </div>

        {failed && <div className="px-2 py-1 text-[10px] text-amber-400/90 shrink-0">{failed}</div>}

        {/* Every tab stays mounted so switching back keeps the page and its
            scroll position; only the active one is visible. */}
        <div className="flex-1 min-h-0 relative" style={{ background: '#0d0d0f' }}>
          {pane.tabs.map((tabUrl, i) => (
            <webview
              key={i}
              ref={
                i === pane.activeTab ? (viewRef as unknown as React.Ref<HTMLElement>) : undefined
              }
              src={tabUrl}
              // Each session browses in its own partition, so logins and cookies
              // in one session's pane don't leak into another's.
              partition={`persist:vorn-browser-${sessionId}`}
              className="absolute inset-0 w-full h-full"
              style={i === pane.activeTab ? undefined : { visibility: 'hidden' }}
            />
          ))}
        </div>
      </PaneCard>
    )
  })
)
