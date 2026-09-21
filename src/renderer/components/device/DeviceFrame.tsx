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
 * The device's body, as Apple draws it.
 *
 * Two shapes of artwork. Most bundles ship one composite picture of the whole
 * body, which is drawn for that device and fits it exactly; the rest ship it
 * in nine pieces, and those are composited the way Apple's own renderer does
 * it rather than the way a CSS border works — the corners at the artwork's
 * natural size, 110 points on a phone, the whole curve and its rim, and the
 * edges keeping that thickness while stretching along their side. Scaling a
 * corner down to the inset the screen sits behind loses the curve entirely and
 * leaves a plain black border, which is what this looked like before.
 *
 * The body sits *behind* the screen, with the buttons behind the body so the
 * rail overlaps them the way a real one does. The screen is then laid on top,
 * inset by the body's thickness and rounded to the device's own radius.
 *
 * Held sideways, the whole body is rotated rather than the pieces reshuffled:
 * the art is drawn portrait, and one transform keeps every curve and every
 * button on the edge it belongs to.
 */
function DeviceBody({ bezel }: { bezel: Bezel }): React.ReactElement | null {
  const chrome = bezel.chrome
  if (!chrome) return null
  const at = (points: number): number => points * bezel.scale
  // The body is laid out portrait, then turned with the device. A quarter-turn
  // swaps the box it occupies; a half-turn does not.
  const outerWidth = bezel.width + bezel.inset.left + bezel.inset.right
  const outerHeight = bezel.height + bezel.inset.top + bezel.inset.bottom
  const quarter = bezel.bodyTurn === 90 || bezel.bodyTurn === -90
  const width = quarter ? outerHeight : outerWidth
  const height = quarter ? outerWidth : outerHeight
  const piece = (url: string, style: React.CSSProperties): React.ReactElement => (
    <span
      aria-hidden
      className="absolute"
      style={{
        ...style,
        backgroundImage: `url(${url})`,
        backgroundSize: '100% 100%',
        backgroundRepeat: 'no-repeat'
      }}
    />
  )
  const i = chrome.images
  const corner = i && { width: at(i.topLeft.width), height: at(i.topLeft.height) }
  return (
    <div
      aria-hidden
      className="absolute pointer-events-none"
      style={{
        width,
        height,
        left: (outerWidth - width) / 2,
        top: (outerHeight - height) / 2,
        transform: bezel.bodyTurn ? `rotate(${bezel.bodyTurn}deg)` : undefined
      }}
    >
      {chrome.buttons.map((b, n) => {
        const vertical = b.side === 'left' || b.side === 'right'
        return (
          <span
            key={`${b.name}-${n}`}
            className="absolute"
            style={{
              [b.side]: -at(b.out),
              [vertical ? 'top' : 'left']: at(b.along),
              width: at(b.width),
              height: at(b.height),
              backgroundImage: `url(${b.url})`,
              backgroundSize: '100% 100%',
              backgroundRepeat: 'no-repeat'
            }}
          />
        )
      })}
      {/* One picture of the body where the bundle has one — see
          `device-chrome.ts` for why that is the artwork to trust. */}
      {chrome.composite && piece(chrome.composite.url, { inset: 0 })}
      {i && corner && (
        <>
          {piece(i.topLeft.url, { left: 0, top: 0, ...corner })}
          {piece(i.topRight.url, { right: 0, top: 0, ...corner })}
          {piece(i.bottomLeft.url, { left: 0, bottom: 0, ...corner })}
          {piece(i.bottomRight.url, { right: 0, bottom: 0, ...corner })}
          {piece(i.top.url, {
            left: corner.width,
            right: corner.width,
            top: 0,
            height: at(i.top.height)
          })}
          {piece(i.bottom.url, {
            left: corner.width,
            right: corner.width,
            bottom: 0,
            height: at(i.bottom.height)
          })}
          {piece(i.left.url, {
            top: corner.height,
            bottom: corner.height,
            left: 0,
            width: at(i.left.width)
          })}
          {piece(i.right.url, {
            top: corner.height,
            bottom: corner.height,
            right: 0,
            width: at(i.right.width)
          })}
        </>
      )}
    </div>
  )
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
 * black". Where the machine has Apple's own artwork for this device it is that
 * device's body; where it does not, it is a plain frame that claims only what
 * the screen's shape can honestly tell us.
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
              paddingLeft: bezel.inset.left,
              paddingRight: bezel.inset.right,
              paddingTop: bezel.inset.top,
              paddingBottom: bezel.inset.bottom,
              borderRadius: bezel.outerRadius,
              // Apple's artwork is the body itself, corners and all, so a
              // surface behind it would show at every rounded edge.
              background: bezel.chrome ? undefined : 'var(--color-surface-overlay)',
              boxShadow: bezel.chrome
                ? '0 12px 32px rgba(0,0,0,0.45)'
                : 'inset 0 0 0 1px rgba(255,255,255,0.07), 0 12px 32px rgba(0,0,0,0.45)'
            }}
          >
            <DeviceBody bezel={bezel} />
            <div
              className={`relative overflow-hidden ${typing ? 'ring-1 ring-sky-400/40' : ''}`}
              style={{
                width: bezel.width,
                height: bezel.height,
                borderRadius: bezel.innerRadius,
                // Above the body: the artwork is a whole slab, and the screen
                // is what Apple lays on top of it.
                zIndex: 1
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
                // lands where it was aimed. Turned, when the device is sideways
                // and the app inside it is not — the picture is then drawn at
                // its own portrait size and rotated about its centre, which
                // leaves the element's bounding box equal to the turned screen.
                style={
                  bezel.turn
                    ? {
                        position: 'absolute',
                        width: bezel.points.width * bezel.scale,
                        height: bezel.points.height * bezel.scale,
                        left: '50%',
                        top: '50%',
                        transform: `translate(-50%, -50%) rotate(${bezel.turn}deg)`
                      }
                    : { width: bezel.width, height: bezel.height }
                }
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
