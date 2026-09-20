import { describe, it, expect } from 'vitest'
import {
  bezelFor,
  fitScale,
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
    expect(bezel.width + 2 * bezel.thickness).toBeLessThanOrEqual(container.width)
    expect(bezel.height + 2 * bezel.thickness).toBeLessThanOrEqual(container.height)
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

  it('draws only the buttons it can honestly place', () => {
    // An iPad's volume keys move from model to model, so the frame claims only
    // the one button every tablet has.
    expect(bezelFor(IPAD, { width: 900, height: 900 }, 'fit').buttons).toHaveLength(1)
    expect(bezelFor(IPHONE, { width: 500, height: 900 }, 'fit').buttons.length).toBeGreaterThan(1)
  })

  it('puts the buttons on the edges that are long, whichever way up it is', () => {
    const portrait = bezelFor(IPHONE, { width: 500, height: 900 }, 'fit')
    expect(portrait.buttons.every((b) => b.side === 'left' || b.side === 'right')).toBe(true)
    const landscape = bezelFor({ width: 874, height: 402 }, { width: 900, height: 500 }, 'fit')
    expect(landscape.buttons.every((b) => b.side === 'top' || b.side === 'bottom')).toBe(true)
  })

  it('stays drawable in a pane barely bigger than nothing', () => {
    const bezel = bezelFor(IPHONE, { width: 40, height: 40 }, 'fit')
    expect(bezel.thickness).toBeGreaterThanOrEqual(6)
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
