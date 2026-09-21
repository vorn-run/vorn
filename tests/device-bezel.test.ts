import { describe, it, expect } from 'vitest'
import {
  bezelFor,
  fitScale,
  screenPointFor,
  maxEdgeFor,
  steppedZoom,
  PANE_MAX_EDGE,
  ZOOM_MAX,
  ZOOM_MIN
} from '../src/renderer/lib/device-bezel'

/**
 * The arithmetic the device pane's tap accuracy rests on.
 *
 * The pane draws the screen at an exact size rather than letting it letterbox,
 * because that is what makes `toPoints` a plain division by the scale. If the
 * drawn box ever stops being `points × scale`, taps land somewhere other than
 * where they were aimed and nothing on screen says so.
 */

const IPHONE = { width: 402, height: 874 }
const IPAD = { width: 1024, height: 1366 }

describe('drawing the screen', () => {
  it('draws exactly points × scale, so there is no letterbox to correct for', () => {
    const bezel = bezelFor(IPHONE, { width: 500, height: 900 }, 1)
    expect(bezel.width).toBe(402)
    expect(bezel.height).toBe(874)
    expect(bezel.scale).toBe(1)
  })

  it('keeps that promise when the device is turned sideways', () => {
    const bezel = bezelFor({ width: 874, height: 402 }, { width: 1000, height: 600 }, 1)
    expect([bezel.width, bezel.height]).toEqual([874, 402])
    expect(bezel.landscape).toBe(true)
  })

  it('fits by whichever axis runs out first', () => {
    // Tall pane, so the width is the constraint.
    const wide = bezelFor(IPHONE, { width: 300, height: 2000 }, 'fit')
    expect(wide.width).toBeLessThanOrEqual(300)
    // Short pane, so the height is.
    const tall = bezelFor(IPHONE, { width: 2000, height: 400 }, 'fit')
    expect(tall.height).toBeLessThanOrEqual(400)
  })

  it('leaves room for the frame it is going to draw around it', () => {
    const container = { width: 500, height: 900 }
    const bezel = bezelFor(IPHONE, container, 'fit')
    expect(bezel.width + bezel.inset.left + bezel.inset.right).toBeLessThanOrEqual(container.width)
    expect(bezel.height + bezel.inset.top + bezel.inset.bottom).toBeLessThanOrEqual(
      container.height
    )
  })

  it('falls back to actual size before anything has been laid out', () => {
    // jsdom lays nothing out and the first render has no box either; a scale of
    // zero would render an invisible device and read as a broken pane.
    expect(fitScale(IPHONE, { width: 0, height: 0 }, 8)).toBe(1)
  })

  it('holds the zoom inside its bounds', () => {
    expect(bezelFor(IPHONE, { width: 500, height: 900 }, 99).scale).toBe(ZOOM_MAX)
    expect(bezelFor(IPHONE, { width: 500, height: 900 }, 0.01).scale).toBe(ZOOM_MIN)
  })
})

describe('the frame', () => {
  it('knows a phone from a tablet by shape alone, in either orientation', () => {
    expect(bezelFor(IPHONE, { width: 500, height: 900 }, 'fit').isPhone).toBe(true)
    expect(bezelFor({ width: 874, height: 402 }, { width: 900, height: 500 }, 'fit').isPhone).toBe(
      true
    )
    expect(bezelFor(IPAD, { width: 900, height: 900 }, 'fit').isPhone).toBe(false)
  })

  it('stays drawable in a pane barely bigger than nothing', () => {
    const bezel = bezelFor(IPHONE, { width: 40, height: 40 }, 'fit')
    expect(bezel.inset.left).toBeGreaterThanOrEqual(6)
    expect(bezel.innerRadius).toBeGreaterThan(0)
    expect(bezel.width).toBeGreaterThan(0)
  })
})

describe('what to ask main for', () => {
  it('asks for the pixels it will actually show', () => {
    expect(maxEdgeFor(IPHONE, 1, 2)).toBe(1748)
  })

  it('asks for less when the device is drawn smaller', () => {
    expect(maxEdgeFor(IPHONE, 0.5, 2)).toBeLessThan(maxEdgeFor(IPHONE, 1, 2))
  })

  it('never asks for more than main would send', () => {
    // Above the raw capture main skips the downscale entirely and a ~2.9MB PNG
    // crosses IPC twice a second for an image no bigger on screen.
    expect(maxEdgeFor(IPHONE, 4, 3)).toBe(PANE_MAX_EDGE)
  })
})

describe('stepping the zoom', () => {
  it('moves by a visible amount, and stops at the ends', () => {
    expect(steppedZoom(1, 1)).toBeGreaterThan(1)
    expect(steppedZoom(1, -1)).toBeLessThan(1)
    expect(steppedZoom(ZOOM_MAX, 1)).toBe(ZOOM_MAX)
    expect(steppedZoom(ZOOM_MIN, -1)).toBe(ZOOM_MIN)
  })
})

describe("Apple's own faceplate, when the machine has it", () => {
  /** The shape of a real chrome bundle: phone11, which is the iPhone 18 Pro. */
  const corner = (url: string) => ({ url, width: 110, height: 110 })
  const CHROME = {
    id: 'phone11',
    inset: { left: 18, right: 18, top: 18, bottom: 22 },
    cornerRadius: 80,
    images: {
      topLeft: corner('data:,tl'),
      top: { url: 'data:,t', width: 1, height: 110 },
      topRight: corner('data:,tr'),
      right: { url: 'data:,r', width: 110, height: 1 },
      bottomRight: corner('data:,br'),
      bottom: { url: 'data:,b', width: 1, height: 110 },
      bottomLeft: corner('data:,bl'),
      left: { url: 'data:,l', width: 110, height: 1 }
    },
    buttons: [
      {
        name: 'power',
        url: 'data:,p',
        width: 16,
        height: 101,
        side: 'right' as const,
        out: 8,
        top: 262
      }
    ]
  }

  it('takes the body and the corner radius from the device itself', () => {
    const bezel = bezelFor(IPHONE, { width: 900, height: 1600 }, 1, CHROME)
    expect(bezel.inset).toEqual({ left: 18, right: 18, top: 18, bottom: 22 })
    expect(bezel.outerRadius).toBe(80)
  })

  it('grows the body with the device, the way Simulator does', () => {
    const half = bezelFor(IPHONE, { width: 900, height: 1600 }, 0.5, CHROME)
    expect(half.inset.left).toBe(9)
    expect(half.outerRadius).toBe(40)
  })

  it('fits the whole body, not just the screen', () => {
    const container = { width: 300, height: 700 }
    const bezel = bezelFor(IPHONE, container, 'fit', CHROME)
    expect(bezel.width + bezel.inset.left + bezel.inset.right).toBeLessThanOrEqual(container.width)
    expect(bezel.height + bezel.inset.top + bezel.inset.bottom).toBeLessThanOrEqual(
      container.height
    )
  })

  it('turns the thicknesses exactly as far as the artwork turns', () => {
    // The body is drawn portrait and rotated into place, so each thickness
    // moves with it. A quarter-turn anticlockwise puts the portrait top along
    // the left and the chin — the thicker edge — along the right. If this and
    // the transform on the artwork disagree, the frame reserves space on one
    // edge and paints the body on another.
    const left = bezelFor({ width: 874, height: 402 }, { width: 1200, height: 700 }, 1, CHROME)
    expect(left.bodyTurn).toBe(-90)
    expect(left.inset).toEqual({ left: 18, top: 18, right: 22, bottom: 18 })

    const right = bezelFor(
      { width: 874, height: 402 },
      { width: 1200, height: 700 },
      1,
      CHROME,
      'landscape-right'
    )
    expect(right.bodyTurn).toBe(90)
    expect(right.inset).toEqual({ left: 22, top: 18, right: 18, bottom: 18 })
  })

  it('leaves the body upright for a device that is', () => {
    expect(bezelFor(IPHONE, { width: 900, height: 1600 }, 1, CHROME).bodyTurn).toBe(0)
  })
})

describe('a device held sideways', () => {
  const PANE = { width: 1200, height: 800 }

  it('turns the picture when the app inside it did not', () => {
    // The Home Screen does not rotate on an iPhone: the device goes sideways
    // and the framebuffer stays portrait. Simulator shows that as a turned
    // device with an upright picture, and so must the pane — nothing in the
    // screenshot says the device moved at all.
    const bezel = bezelFor(IPHONE, PANE, 1, null, 'landscape-left')
    expect(bezel.turn).toBe(-90)
    expect([bezel.width, bezel.height]).toEqual([874, 402])
    expect(bezel.landscape).toBe(true)
    expect(bezel.points).toEqual(IPHONE)
  })

  it('turns it the other way round', () => {
    expect(bezelFor(IPHONE, PANE, 1, null, 'landscape-right').turn).toBe(90)
  })

  it('leaves a picture that already rotated alone', () => {
    // An app that rotates hands back landscape pixels; turning those again
    // would stand the screen on its head.
    const bezel = bezelFor({ width: 874, height: 402 }, PANE, 1, null, 'landscape-left')
    expect(bezel.turn).toBe(0)
    expect([bezel.width, bezel.height]).toEqual([874, 402])
  })

  it('stays put in portrait', () => {
    expect(bezelFor(IPHONE, PANE, 1, null, 'portrait').turn).toBe(0)
  })
})

describe('a click on the screen', () => {
  it('divides by the scale, with no letterbox to correct for', () => {
    const bezel = bezelFor(IPHONE, { width: 1200, height: 2000 }, 2)
    expect(screenPointFor(100, 300, bezel)).toEqual({ x: 50, y: 150 })
  })

  it('turns back with the picture', () => {
    // Turned a quarter anticlockwise, the device's top-right corner is what
    // sits at the top-left of the pane. A tap that does not turn back lands
    // somewhere else entirely, and nothing on screen says so.
    const bezel = bezelFor(IPHONE, { width: 1200, height: 800 }, 1, null, 'landscape-left')
    expect(screenPointFor(0, 0, bezel)).toEqual({ x: 402, y: 0 })
    expect(screenPointFor(bezel.width / 2, bezel.height / 2, bezel)).toEqual({ x: 201, y: 437 })
    expect(screenPointFor(bezel.width, bezel.height, bezel)).toEqual({ x: 0, y: 874 })
  })

  it('turns back the other way too', () => {
    const bezel = bezelFor(IPHONE, { width: 1200, height: 800 }, 1, null, 'landscape-right')
    expect(screenPointFor(0, 0, bezel)).toEqual({ x: 0, y: 874 })
  })

  it('refuses a click outside the screen', () => {
    const bezel = bezelFor(IPHONE, { width: 1200, height: 2000 }, 1)
    expect(screenPointFor(-4, 10, bezel)).toBeNull()
    expect(screenPointFor(10, 900, bezel)).toBeNull()
  })
})
