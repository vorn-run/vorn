import { describe, it, expect } from 'vitest'
import { paletteToCss, colorToCss } from '../src/renderer/lib/block-log'

/**
 * xterm reports colour as a palette index or packed RGB. Resolving it wrongly
 * is the difference between a block that matches the live terminal and one that
 * quietly recolours every tool's output.
 */

describe('paletteToCss', () => {
  it('takes the first 16 from the terminal theme', () => {
    expect(paletteToCss(1)).toBe('#ef4444')
    expect(paletteToCss(15)).toBe('#fafafa')
  })

  it('maps the 6x6x6 cube', () => {
    // 16 is the cube's origin, 231 its far corner.
    expect(paletteToCss(16)).toBe('#000000')
    expect(paletteToCss(231)).toBe('#ffffff')
    // 16 + 36*1 + 6*2 + 3 -> r=95, g=135, b=175
    expect(paletteToCss(16 + 36 + 12 + 3)).toBe('#5f87af')
  })

  it('maps the greyscale ramp', () => {
    expect(paletteToCss(232)).toBe('#080808')
    expect(paletteToCss(255)).toBe('#eeeeee')
  })

  it('falls back rather than emitting an invalid colour', () => {
    expect(paletteToCss(-1)).toBe('#d4d4d8')
  })
})

describe('colorToCss', () => {
  it('uses the caller fallback for a default cell', () => {
    expect(colorToCss({ kind: 'default' }, '#d4d4d8')).toBe('#d4d4d8')
  })

  it('unpacks truecolor', () => {
    expect(colorToCss({ kind: 'rgb', value: 0x5391fe }, '#000')).toBe('#5391fe')
  })

  it('pads each channel, so a dark colour is not truncated', () => {
    // 0x000508 must not collapse to #58.
    expect(colorToCss({ kind: 'rgb', value: 0x000508 }, '#000')).toBe('#000508')
  })

  it('routes a palette index through the palette', () => {
    expect(colorToCss({ kind: 'palette', index: 2 }, '#000')).toBe('#22c55e')
  })
})
