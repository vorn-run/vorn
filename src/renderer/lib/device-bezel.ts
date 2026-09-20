/**
 * How big to draw a simulator's screen, and the frame around it.
 *
 * Two jobs, kept in one pure module because they are the same arithmetic seen
 * from either end: the screen is drawn at an exact size in CSS pixels, and the
 * bezel is padding around it.
 *
 * Drawing the screen at an exact size — rather than letting `object-contain`
 * letterbox it inside whatever box it lands in — is what keeps `toPoints` in
 * `DeviceCard` correct at every zoom level. With `width = points × scale`, that
 * function's letterbox terms are zero and its `min()` is the scale itself, so
 * a tap lands where it was aimed whether the pane is showing the device at 40%
 * or at 1:1. Nothing else in the pane fails so quietly when it is wrong.
 *
 * Nothing here knows a device by name: an iPad and a phone differ only by the
 * aspect ratio of the screen they report, which is also all that tells us
 * whether drawing side buttons would be a lie.
 */

export interface ScreenSize {
  width: number
  height: number
}

/** A switch or button moulded into the frame, as fractions of the long edge. */
export interface BezelButton {
  side: 'left' | 'right' | 'top' | 'bottom'
  /** Distance from the device's top edge, as a fraction of the long edge. */
  start: number
  /** How long the button is, as a fraction of the long edge. */
  length: number
}

export interface Bezel {
  /** CSS pixels per device point. The zoom, in the only unit that matters. */
  scale: number
  /** The drawn screen, in CSS pixels. */
  width: number
  height: number
  thickness: number
  outerRadius: number
  innerRadius: number
  landscape: boolean
  isPhone: boolean
  buttons: BezelButton[]
}

/** Zoom bounds. Past 4× there is no more detail to see — only bigger pixels. */
export const ZOOM_MIN = 0.25
export const ZOOM_MAX = 4
/** One click of the zoom buttons. */
export const ZOOM_STEP = 1.25

/**
 * The most pixels the pane will ever ask main for.
 *
 * Mirrors `HARD_MAX_EDGE` in `src/main/device-registry.ts`, which cannot be
 * imported here. Asking for more is worse than useless: above the raw capture
 * main skips the downscale entirely and a ~2.9 MB PNG crosses IPC twice a
 * second for an image no larger on screen.
 */
export const PANE_MAX_EDGE = 2000

/** Long edge ÷ short edge at which a screen stops being a tablet's. */
const PHONE_ASPECT = 1.8

/** Room left around the frame so a protruding side button is not clipped. */
const GUTTER = 8

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high)
}

/**
 * Where the buttons sit, as fractions of the device's long edge.
 *
 * Taken from where they are on the hardware rather than from a screenshot, and
 * only drawn on a phone: an iPad's volume keys are on a different edge per
 * model, and a frame that puts them in the wrong place is worse than a frame
 * with none.
 */
function buttonsFor(isPhone: boolean, landscape: boolean): BezelButton[] {
  // Turned a quarter, the device's left edge is the one at the top.
  const left = landscape ? 'top' : 'left'
  const right = landscape ? 'bottom' : 'right'
  const power: BezelButton = { side: right, start: 0.24, length: 0.09 }
  if (!isPhone) return [power]
  return [
    { side: left, start: 0.14, length: 0.04 },
    { side: left, start: 0.21, length: 0.06 },
    { side: left, start: 0.3, length: 0.06 },
    power
  ]
}

/**
 * The scale at which the whole screen, and its frame, fit the pane.
 *
 * Falls back to 1 when the container has not been laid out yet — which is
 * every first render, and every render under a test environment that performs
 * no layout. Zero would render nothing and read as a broken pane.
 */
export function fitScale(screen: ScreenSize, container: ScreenSize, thickness: number): number {
  const inset = 2 * thickness + 2 * GUTTER
  const availableWidth = container.width - inset
  const availableHeight = container.height - inset
  if (!(screen.width > 0 && screen.height > 0)) return 1
  if (!(availableWidth > 0 && availableHeight > 0)) return 1
  return Math.min(availableWidth / screen.width, availableHeight / screen.height)
}

/**
 * Frame thickness, derived from the pane rather than from the drawn screen.
 *
 * The drawn screen depends on the space the frame leaves, so taking the
 * thickness from it would be circular. The pane is the one measurement that
 * does not move when the zoom does, which also means the frame keeps its
 * weight while the device grows and shrinks inside it.
 */
export function bezelThickness(container: ScreenSize, isPhone: boolean): number {
  const shortest = Math.min(container.width, container.height)
  return clamp(Math.round(shortest * (isPhone ? 0.035 : 0.028)), 6, 16)
}

/** Everything needed to draw one device, at one zoom, in one pane. */
export function bezelFor(screen: ScreenSize, container: ScreenSize, zoom: number | 'fit'): Bezel {
  const width = Math.max(screen.width, 0)
  const height = Math.max(screen.height, 0)
  const long = Math.max(width, height)
  const short = Math.min(width, height)
  // Long over short, so a phone held sideways is still a phone.
  const isPhone = short > 0 ? long / short >= PHONE_ASPECT : true
  const landscape = width > height
  const thickness = bezelThickness(container, isPhone)
  const scale =
    zoom === 'fit'
      ? fitScale({ width, height }, container, thickness)
      : clamp(zoom, ZOOM_MIN, ZOOM_MAX)
  const drawnWidth = Math.round(width * scale)
  const drawnHeight = Math.round(height * scale)
  const outerRadius = Math.round(Math.min(drawnWidth, drawnHeight) * (isPhone ? 0.085 : 0.035))
  return {
    scale,
    width: drawnWidth,
    height: drawnHeight,
    thickness,
    outerRadius,
    // Concentric with the outer one, never negative on a small drawing.
    innerRadius: Math.max(outerRadius - thickness, 2),
    landscape,
    isPhone,
    buttons: buttonsFor(isPhone, landscape)
  }
}

/**
 * The longest edge, in image pixels, worth asking main for.
 *
 * Follows the drawn size rather than the pane's, so zooming out makes the poll
 * cheaper and zooming in buys real detail — up to the point where more pixels
 * would only cost bandwidth.
 */
export function maxEdgeFor(screen: ScreenSize, scale: number, dpr: number): number {
  const longest = Math.max(screen.width, screen.height)
  return Math.ceil(Math.min(longest * scale * (dpr > 0 ? dpr : 1), PANE_MAX_EDGE))
}

/** One click of zoom out of, or further into, the current scale. */
export function steppedZoom(current: number, direction: 1 | -1): number {
  const next = direction === 1 ? current * ZOOM_STEP : current / ZOOM_STEP
  return clamp(Number(next.toFixed(3)), ZOOM_MIN, ZOOM_MAX)
}
