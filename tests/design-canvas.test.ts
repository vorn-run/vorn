import { describe, it, expect } from 'vitest'
import {
  CANVAS_PAD,
  LABEL_HEIGHT,
  MIN_ZOOM,
  artboardUrl,
  fitZoom,
  placeArtboards
} from '../src/renderer/lib/design-canvas'

const BOARDS = [
  { id: 'desktop', label: 'Desktop', width: 1440, height: 900 },
  { id: 'phone', label: 'Phone', width: 390, height: 844 }
]

describe('design canvas layout', () => {
  it('lays artboards side by side at the zoom, labels above', () => {
    const { placed, width, height } = placeArtboards(BOARDS, 0.5)
    expect(placed.map((b) => [b.left, b.top])).toEqual([
      [CANVAS_PAD, CANVAS_PAD + LABEL_HEIGHT],
      [CANVAS_PAD * 2 + 720, CANVAS_PAD + LABEL_HEIGHT]
    ])
    expect(width).toBe(CANVAS_PAD * 3 + 720 + 195)
    expect(height).toBe(CANVAS_PAD * 2 + LABEL_HEIGHT + 450)
  })

  it('fits every artboard in the area, never below what a guest can zoom to', () => {
    expect(fitZoom(BOARDS, { width: 1000, height: 800 })).toBe(0.5)
    expect(fitZoom(BOARDS, { width: 300, height: 800 })).toBe(MIN_ZOOM)
    expect(fitZoom(BOARDS, { width: 9000, height: 9000 })).toBe(1)
  })

  it('tells each artboard its id through the hash, keeping the query', () => {
    expect(artboardUrl('http://127.0.0.1:9/artifact/a1/2?t=tok', 'phone')).toBe(
      'http://127.0.0.1:9/artifact/a1/2?t=tok#artboard=phone'
    )
    expect(artboardUrl('file:///repo/hero.dc.html#board-2', 'dark')).toBe(
      'file:///repo/hero.dc.html#artboard=dark'
    )
  })
})
