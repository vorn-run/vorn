import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Maximize, Minus, Plus } from 'lucide-react'
import type { ArtifactArtboard } from '../../../shared/types'
import {
  LABEL_HEIGHT,
  artboardUrl,
  clampZoom,
  fitZoom,
  placeArtboards
} from '../../lib/design-canvas'
import { Tooltip } from '../Tooltip'

interface GuestElement extends HTMLElement {
  getWebContentsId(): number
  setZoomFactor(factor: number): void
  reload(): void
}

export interface CanvasPin {
  id: string
  artboard: string
  x: number
  y: number
  n: number
  state: 'draft' | 'sent' | 'focus'
}

interface Props {
  sessionId: string
  partition: string
  /** The design's own address; each artboard loads it with its id in the hash. */
  url: string
  artboards: ArtifactArtboard[]
  selected: string
  onSelect: (id: string) => void
  /** Values the person set, put into each artboard as it loads. */
  tweaks: Record<string, unknown>
  /** Changes when the design's file does, so every artboard repaints. */
  reloadKey: number
  pinning: boolean
  pins: CanvasPin[]
  /** A click on an artboard while pinning: its own pixels, and where on the canvas it landed. */
  onPoint: (artboard: string, point: { x: number; y: number }, at: { x: number; y: number }) => void
  onPinClick: (id: string) => void
}

/** One artboard's live guest, sized to its frame and zoomed so its page lays out at full size. */
function Artboard({
  sessionId,
  partition,
  src,
  board,
  zoom,
  tweaks,
  reloadKey
}: {
  sessionId: string
  partition: string
  src: string
  board: ArtifactArtboard
  zoom: number
  tweaks: Record<string, unknown>
  reloadKey: number
}): React.JSX.Element {
  const ref = useRef<GuestElement | null>(null)
  const ready = useRef(false)
  const zoomRef = useRef(zoom)
  const tweaksRef = useRef(tweaks)
  useLayoutEffect(() => {
    zoomRef.current = zoom
    tweaksRef.current = tweaks
  })

  useEffect(() => {
    const view = ref.current
    if (!view) return
    const onReady = (): void => {
      ready.current = true
      try {
        view.setZoomFactor(zoomRef.current)
        window.api.attachArtboard(sessionId, board.id, view.getWebContentsId())
      } catch {
        return
      }
    }
    const onLoaded = (): void => {
      void window.api.setArtboardTweaks(sessionId, board.id, tweaksRef.current).catch(() => {})
    }
    view.addEventListener('dom-ready', onReady)
    view.addEventListener('did-stop-loading', onLoaded)
    return () => {
      view.removeEventListener('dom-ready', onReady)
      view.removeEventListener('did-stop-loading', onLoaded)
      window.api.detachArtboard(sessionId, board.id)
    }
  }, [sessionId, board.id])

  useEffect(() => {
    if (!ready.current) return
    try {
      ref.current?.setZoomFactor(zoom)
    } catch {
      // Not attached yet; dom-ready sets it.
    }
  }, [zoom])

  const firstLoad = useRef(reloadKey)
  useEffect(() => {
    if (reloadKey !== firstLoad.current && ready.current) ref.current?.reload()
  }, [reloadKey])

  return (
    <webview
      ref={ref as unknown as React.Ref<HTMLElement>}
      src={src}
      partition={partition}
      style={{ width: '100%', height: '100%' }}
    />
  )
}

/** A design's artboards side by side on a canvas that pans and zooms. */
export function DesignCanvas({
  sessionId,
  partition,
  url,
  artboards,
  selected,
  onSelect,
  tweaks,
  reloadKey,
  pinning,
  pins,
  onPoint,
  onPinClick
}: Props): React.JSX.Element {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const [zoom, setZoom] = useState(0.5)
  const boardsKey = artboards.map((b) => `${b.id}:${b.width}x${b.height}`).join('|')

  const fit = useCallback(() => {
    const area = scrollRef.current?.getBoundingClientRect()
    if (area) setZoom(fitZoom(artboards, area))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [boardsKey])
  useLayoutEffect(fit, [fit])

  const { placed, width, height } = placeArtboards(artboards, zoom)
  const byId = new Map(placed.map((b) => [b.id, b]))

  // Drag the canvas itself to pan; the artboards keep their own pointer events.
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null)

  return (
    <div className="absolute inset-0 z-[5] flex flex-col min-h-0 bg-surface-sunken">
      <div
        ref={scrollRef}
        data-testid="design-canvas"
        className="relative flex-1 min-h-0 overflow-auto cursor-grab active:cursor-grabbing"
        style={{
          backgroundImage: 'radial-gradient(rgba(255,255,255,0.10) 1px, transparent 1px)',
          backgroundSize: '16px 16px'
        }}
        onPointerDown={(e) => {
          if (e.target !== e.currentTarget && !(e.target as HTMLElement).dataset.canvasBg) return
          const el = scrollRef.current
          if (!el) return
          drag.current = { x: e.clientX, y: e.clientY, left: el.scrollLeft, top: el.scrollTop }
          e.currentTarget.setPointerCapture(e.pointerId)
        }}
        onPointerMove={(e) => {
          const d = drag.current
          const el = scrollRef.current
          if (!d || !el) return
          el.scrollLeft = d.left - (e.clientX - d.x)
          el.scrollTop = d.top - (e.clientY - d.y)
        }}
        onPointerUp={() => (drag.current = null)}
      >
        <div data-canvas-bg="1" className="relative" style={{ width, height }}>
          {placed.map((b) => {
            const w = Math.round(b.width * zoom)
            const h = Math.round(b.height * zoom)
            const isSelected = b.id === selected
            return (
              <div key={b.id}>
                <button
                  type="button"
                  onClick={() => onSelect(b.id)}
                  aria-pressed={isSelected}
                  className="absolute flex items-baseline gap-1.5 font-mono text-[11px] text-ink-faint
                             whitespace-nowrap hover:text-ink-secondary"
                  style={{ left: b.left, top: b.top - LABEL_HEIGHT, height: LABEL_HEIGHT - 4 }}
                >
                  <span className={isSelected ? 'text-ink' : 'text-ink-secondary'}>{b.label}</span>
                  <span>
                    {b.width} × {b.height}
                  </span>
                </button>
                <div
                  className="absolute bg-white"
                  style={{
                    left: b.left,
                    top: b.top,
                    width: w,
                    height: h,
                    outline: isSelected ? '2px solid var(--color-status-blue)' : undefined,
                    outlineOffset: 2
                  }}
                  onPointerDown={() => onSelect(b.id)}
                >
                  <Artboard
                    sessionId={sessionId}
                    partition={partition}
                    src={artboardUrl(url, b.id)}
                    board={b}
                    zoom={zoom}
                    tweaks={tweaks}
                    reloadKey={reloadKey}
                  />
                  {pinning && (
                    <div
                      aria-label={`Pin a comment on ${b.label}`}
                      className="absolute inset-0 cursor-crosshair"
                      onClick={(e) => {
                        const box = e.currentTarget.getBoundingClientRect()
                        const area = scrollRef.current!.getBoundingClientRect()
                        onSelect(b.id)
                        onPoint(
                          b.id,
                          {
                            x: Math.round((e.clientX - box.left) / zoom),
                            y: Math.round((e.clientY - box.top) / zoom)
                          },
                          { x: e.clientX - area.left, y: e.clientY - area.top }
                        )
                      }}
                    />
                  )}
                </div>
              </div>
            )
          })}
          {pins.map((p) => {
            const b = byId.get(p.artboard)
            if (!b) return null
            return (
              <button
                key={p.id}
                type="button"
                onClick={() => onPinClick(p.id)}
                aria-label={`Comment ${p.n} on ${b.label}`}
                className={`absolute z-10 grid place-items-center w-5 h-5 font-mono text-[10px] font-bold
                           text-surface-base ${p.state === 'sent' ? 'bg-status-slate' : 'bg-status-blue'}
                           ${p.state === 'focus' ? 'ring-2 ring-ink' : ''}`}
                style={{
                  left: b.left + Math.round(p.x * zoom),
                  top: b.top + Math.round(p.y * zoom) - 20,
                  borderRadius: '50% 50% 50% 0'
                }}
              >
                {p.n}
              </button>
            )
          })}
        </div>
      </div>
      <div
        className="absolute left-2.5 bottom-2.5 z-10 flex items-center gap-0.5 p-0.5 rounded
                   border border-white/[0.12] font-mono text-[11.5px] text-ink-secondary"
        style={{ background: 'var(--color-surface-overlay)' }}
      >
        <Tooltip label="Zoom out" position="top">
          <button
            type="button"
            aria-label="Zoom out"
            onClick={() => setZoom((z) => clampZoom(z - 0.1))}
            className="grid place-items-center w-6 h-6 rounded hover:bg-white/[0.06]"
          >
            <Minus size={12} strokeWidth={2} />
          </button>
        </Tooltip>
        <span className="px-1.5 text-ink tabular-nums" aria-label="Zoom">
          {Math.round(zoom * 100)}%
        </span>
        <Tooltip label="Zoom in" position="top">
          <button
            type="button"
            aria-label="Zoom in"
            onClick={() => setZoom((z) => clampZoom(z + 0.1))}
            className="grid place-items-center w-6 h-6 rounded hover:bg-white/[0.06]"
          >
            <Plus size={12} strokeWidth={2} />
          </button>
        </Tooltip>
        <Tooltip label="Fit every artboard" position="top">
          <button
            type="button"
            aria-label="Fit every artboard"
            onClick={fit}
            className="grid place-items-center w-6 h-6 rounded hover:bg-white/[0.06]"
          >
            <Maximize size={12} strokeWidth={2} />
          </button>
        </Tooltip>
      </div>
    </div>
  )
}
