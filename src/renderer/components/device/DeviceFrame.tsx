import type { MutableRefObject, PointerEvent as ReactPointerEvent, MouseEvent } from 'react'
import { PANE_SURFACE } from '../../lib/pane-surface'
import type { Bezel } from '../../lib/device-bezel'

interface Props {
  sessionId: string
  /** For the image's alt text: what the person is looking at. */
  name: string
  /** Base64 PNG, or null before the first frame. */
  frame: string | null
  /** How to draw it, or null until the screen size is known. */
  bezel: Bezel | null
  /** The poll is failing: the picture on screen is stale. */
  stale: boolean
  /** The picker is armed, so a click describes rather than taps. */
  picking: boolean
  /** Keys typed here are going to the device. */
  typing: boolean
  annotating: boolean
  containerRef: MutableRefObject<HTMLDivElement | null>
  imgRef: MutableRefObject<HTMLImageElement | null>
  inkRef: MutableRefObject<HTMLCanvasElement | null>
  emptyLabel: string
  onClickScreen: (e: MouseEvent<HTMLImageElement>) => void
  onInkDown: (e: ReactPointerEvent<HTMLCanvasElement>) => void
  onInkMove: (e: ReactPointerEvent<HTMLCanvasElement>) => void
  onInkUp: () => void
  onStageKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => void
  onStagePointerDown: () => void
  onStageBlur: (e: React.FocusEvent<HTMLDivElement>) => void
}

/**
 * The screen, in a frame.
 *
 * Presentational: it makes no calls and holds no state, so everything that
 * talks to main still lives in one file. What it does own is the one structural
 * promise the tap arithmetic depends on — the image is drawn at exactly
 * `bezel.width × bezel.height`, and the frame is padding around it. There is no
 * box for the picture to letterbox inside, at any zoom.
 *
 * The frame earns its place beyond looking like a device: a screenshot that
 * ends flush against a dark pane reads as a picture pasted into the app, and
 * gives a person no edge to tell "the device stops here" from "the app drew
 * black". The side buttons are drawn only where they honestly are — a phone —
 * because an iPad's are in a different place on every model.
 */
export function DeviceFrame({
  sessionId,
  name,
  frame,
  bezel,
  stale,
  picking,
  typing,
  annotating,
  containerRef,
  imgRef,
  inkRef,
  emptyLabel,
  onClickScreen,
  onInkDown,
  onInkMove,
  onInkUp,
  onStageKeyDown,
  onStagePointerDown,
  onStageBlur
}: Props): React.ReactElement {
  return (
    <div
      ref={containerRef}
      data-testid={`device-pane-${sessionId}`}
      // Focusable so typing has somewhere to land without a listener on the
      // window: focus is the scope, and clicking anywhere else ends it.
      tabIndex={0}
      onKeyDown={onStageKeyDown}
      onPointerDown={onStagePointerDown}
      onBlur={onStageBlur}
      className="flex-1 min-h-0 min-w-0 relative overflow-auto outline-none"
      style={{ background: PANE_SURFACE }}
    >
      {/* Centres the device while it fits, and lets it be scrolled to when it
          does not. Plain `items-center` on the scrolling box would push the top
          of a zoomed-in device out of reach above the scroll origin. */}
      <div className="min-w-full min-h-full w-max h-max flex items-center justify-center p-2">
        {frame && bezel ? (
          <div
            className="relative shrink-0"
            style={{
              padding: bezel.thickness,
              borderRadius: bezel.outerRadius,
              background: 'var(--color-surface-overlay)',
              boxShadow: `inset 0 0 0 1px rgba(255,255,255,0.07), 0 12px 32px rgba(0,0,0,0.45)`
            }}
          >
            {bezel.buttons.map((b, i) => {
              const along = bezel.landscape ? bezel.width : bezel.height
              const size = Math.max(Math.round(along * b.length), 6)
              const offset = Math.round(along * b.start) + bezel.thickness
              const vertical = b.side === 'left' || b.side === 'right'
              return (
                <span
                  key={i}
                  aria-hidden
                  className="absolute bg-white/[0.12] rounded-sm"
                  style={{
                    [b.side]: -2,
                    [vertical ? 'top' : 'left']: offset,
                    [vertical ? 'width' : 'height']: 2,
                    [vertical ? 'height' : 'width']: size
                  }}
                />
              )
            })}
            <div
              className={`relative overflow-hidden ${typing ? 'ring-1 ring-sky-400/40' : ''}`}
              style={{
                width: bezel.width,
                height: bezel.height,
                borderRadius: bezel.innerRadius
              }}
            >
              <img
                ref={imgRef}
                src={`data:image/png;base64,${frame}`}
                alt={`Screen of ${name}`}
                data-testid={`device-frame-${sessionId}`}
                onClick={onClickScreen}
                // Sized exactly, never letterboxed: `toPoints` reads this box
                // back and divides by the same scale, so a tap at any zoom
                // lands where it was aimed.
                style={{ width: bezel.width, height: bezel.height }}
                // Dimmed once a poll fails: the frame is the last one that
                // arrived, and rendering a dead screen at full strength makes a
                // frozen device look live. The person taps it, every tap throws,
                // and nothing on screen ever said the picture had stopped.
                className={`select-none transition-opacity ${stale ? 'opacity-40' : ''} ${
                  picking ? 'cursor-crosshair' : 'cursor-pointer'
                }`}
                draggable={false}
              />
              {/* Mounted only while armed: a permanent overlay would swallow
                  every tap meant for the device. Inside the screen, not the
                  stage, so ink cannot be drawn onto the frame. */}
              {annotating && (
                <canvas
                  ref={inkRef}
                  data-testid={`device-ink-${sessionId}`}
                  onPointerDown={onInkDown}
                  onPointerMove={onInkMove}
                  onPointerUp={onInkUp}
                  className="absolute inset-0 w-full h-full cursor-crosshair z-10"
                />
              )}
            </div>
          </div>
        ) : (
          <span className="text-[11px] text-gray-500">{emptyLabel}</span>
        )}
      </div>
    </div>
  )
}
