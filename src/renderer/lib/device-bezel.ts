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

import type { DeviceChrome, DeviceOrientation } from '../../shared/types'

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
  /** The screen as drawn, in CSS pixels — turned, when the device is. */
  width: number
  height: number
  /** The device's own screen, in points, whichever way the picture is shown. */
  points: ScreenSize
  /**
   * How far the picture is turned to match a device held sideways.
   *
   * Zero almost always: an app that rotates with the device hands back a
   * landscape picture and there is nothing to do. It is the app that does
   * *not* rotate — the Home Screen above all — that needs this, because the
   * device is sideways while the pixels stay portrait, and Simulator shows
   * exactly that: a turned device with an upright picture inside it.
   */
  turn: -90 | 0 | 90
  /** Each side of the frame, in CSS pixels. Apple's own where we have them. */
  inset: { left: number; right: number; top: number; bottom: number }
  outerRadius: number
  innerRadius: number
  landscape: boolean
  isPhone: boolean
  /** Drawn only when Apple's artwork is not available. */
  buttons: BezelButton[]
  /** The real device body, when the machine has it. */
  chrome: DeviceChrome | null
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
 * The scale at which a screen *and a frame that scales with it* both fit.
 *
 * Apple's insets are in device points, so they take a share of the pane that
 * grows with the zoom — unlike the frame we draw ourselves, whose thickness
 * comes from the pane and stays put.
 */
export function fitScaleWithFrame(
  screen: ScreenSize,
  container: ScreenSize,
  sides: { left: number; right: number; top: number; bottom: number }
): number {
  const width = screen.width + sides.left + sides.right
  const height = screen.height + sides.top + sides.bottom
  const availableWidth = container.width - 2 * GUTTER
  const availableHeight = container.height - 2 * GUTTER
  if (!(width > 0 && height > 0)) return 1
  if (!(availableWidth > 0 && availableHeight > 0)) return 1
  return Math.min(availableWidth / width, availableHeight / height)
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

/**
 * Everything needed to draw one device, at one zoom, in one pane.
 *
 * With `chrome`, the sides and the corner radius are the real device's, taken
 * from the artwork Apple already ships (see `src/main/device-chrome.ts`), and
 * they scale with the screen the way they do in Simulator. Without it — no
 * Xcode, another platform — the frame is drawn from the screen's shape alone,
 * which is close enough to read as a device and never claims a button the
 * device may not have.
 */
export function bezelFor(
  screen: ScreenSize,
  container: ScreenSize,
  zoom: number | 'fit',
  chrome: DeviceChrome | null = null,
  orientation: DeviceOrientation = 'portrait'
): Bezel {
  const points = { width: Math.max(screen.width, 0), height: Math.max(screen.height, 0) }
  // A device held sideways whose picture came back portrait is the one case
  // the pane has to turn itself.
  const sideways = orientation === 'landscape-left' || orientation === 'landscape-right'
  const turn: -90 | 0 | 90 =
    sideways && points.height > points.width ? (orientation === 'landscape-left' ? -90 : 90) : 0
  const width = turn ? points.height : points.width
  const height = turn ? points.width : points.height
  const long = Math.max(width, height)
  const short = Math.min(width, height)
  // Long over short, so a phone held sideways is still a phone.
  const isPhone = short > 0 ? long / short >= PHONE_ASPECT : true
  const landscape = width > height
  const scale =
    zoom === 'fit'
      ? chrome
        ? // Apple's frame is measured in the device's own points, so it grows
          // and shrinks with the screen — the fit has to solve for both at once.
          fitScaleWithFrame({ width, height }, container, sidesOf(chrome, landscape, 1))
        : fitScale({ width, height }, container, bezelThickness(container, isPhone))
      : clamp(zoom, ZOOM_MIN, ZOOM_MAX)
  const drawnWidth = Math.round(width * scale)
  const drawnHeight = Math.round(height * scale)
  const inset = chrome
    ? sidesOf(chrome, landscape, scale)
    : evenSides(bezelThickness(container, isPhone))
  const outerRadius = chrome
    ? Math.round(chrome.cornerRadius * scale)
    : Math.round(Math.min(drawnWidth, drawnHeight) * (isPhone ? 0.085 : 0.035))
  return {
    scale,
    width: drawnWidth,
    height: drawnHeight,
    points,
    turn,
    inset,
    outerRadius,
    // Concentric with the outer one, never negative on a small drawing.
    innerRadius: Math.max(outerRadius - Math.max(inset.left, inset.top), 2),
    landscape,
    isPhone,
    // Apple's frame already has the buttons moulded into it.
    buttons: chrome ? [] : buttonsFor(isPhone, landscape),
    chrome
  }
}

/** The same thickness on every side, for the frame we draw ourselves. */
function evenSides(thickness: number): Bezel['inset'] {
  return { left: thickness, right: thickness, top: thickness, bottom: thickness }
}

/**
 * Apple's insets, turned with the device.
 *
 * The artwork is drawn portrait; held sideways, the body's left edge is the
 * one along the top, and the frame has to follow or the thicker chin ends up
 * on the wrong side of the screen.
 */
function sidesOf(chrome: DeviceChrome, landscape: boolean, scale: number): Bezel['inset'] {
  const at = (value: number): number => Math.max(Math.round(value * scale), 1)
  const { left, right, top, bottom } = chrome.inset
  return landscape
    ? { left: at(bottom), right: at(top), top: at(left), bottom: at(right) }
    : { left: at(left), right: at(right), top: at(top), bottom: at(bottom) }
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

/**
 * A click on the drawn screen → a point on the device.
 *
 * `x` and `y` are measured from the top-left of the drawn screen. The screen is
 * drawn at exactly `points × scale`, so ordinarily this is a division — and
 * when the picture has been turned to match a sideways device, the click is
 * turned back the same amount first. Getting that wrong is the one failure in
 * this pane that leaves no trace: the picture looks right and the tap lands
 * somewhere else entirely.
 *
 * Null for a click outside the screen.
 */
export function screenPointFor(
  x: number,
  y: number,
  bezel: Bezel,
  /** The screen as actually laid out. Measured beats computed here: a rounded
   *  pixel or a stylesheet nobody expected shows up in the box, not in the
   *  zoom, and this is the one mapping that must follow what is on screen. */
  box?: ScreenSize
): { x: number; y: number } | null {
  const { points, turn } = bezel
  if (!(points.width > 0 && points.height > 0)) return null
  // Turned, the drawn box is the screen on its side.
  const drawnWidth = box?.width ?? bezel.width
  const drawnHeight = box?.height ?? bezel.height
  const alongX = turn ? points.height : points.width
  const alongY = turn ? points.width : points.height
  const scale = Math.min(drawnWidth / alongX, drawnHeight / alongY)
  if (!(scale > 0)) return null
  // Centre-origin, because a rotation turns about the centre.
  const fromCentreX = x - drawnWidth / 2
  const fromCentreY = y - drawnHeight / 2
  const [imageX, imageY] =
    turn === -90
      ? [-fromCentreY, fromCentreX]
      : turn === 90
        ? [fromCentreY, -fromCentreX]
        : [fromCentreX, fromCentreY]
  const point = {
    x: imageX / scale + points.width / 2,
    y: imageY / scale + points.height / 2
  }
  if (point.x < 0 || point.y < 0) return null
  if (point.x > points.width || point.y > points.height) return null
  return point
}

/** One click of zoom out of, or further into, the current scale. */
export function steppedZoom(current: number, direction: 1 | -1): number {
  const next = direction === 1 ? current * ZOOM_STEP : current / ZOOM_STEP
  return clamp(Number(next.toFixed(3)), ZOOM_MIN, ZOOM_MAX)
}
