import { describe, it, expect } from 'vitest'
import { fitGrid, MIN_FONT_PX } from '../src/renderer/lib/grid-fit'

/**
 * TP-T9e, the renderer's half: viewers fit, they never clip. Whatever the
 * session's grid and whatever the pane, every column and row is drawn, on
 * screen at a font no lower than 9 px or a pan away, and a pane that holds
 * the grid draws it at the user's font with nothing to pan. The same
 * arithmetic as the size rule's own `fit`, checked there in every state its
 * scenarios pass through.
 */
describe('fitting a grid that is not this pane’s size', () => {
  it('matches the size rule’s numbers', () => {
    expect(fitGrid({ cols: 80, rows: 24 }, { cols: 120, rows: 40 }, 13)).toEqual({
      font: 13,
      shown: { cols: 80, rows: 24 },
      pan: { cols: 0, rows: 0 }
    })
    expect(fitGrid({ cols: 120, rows: 30 }, { cols: 60, rows: 40 }, 13)).toEqual({
      font: 9,
      shown: { cols: 86, rows: 30 },
      pan: { cols: 34, rows: 0 }
    })
    const scaled = fitGrid({ cols: 100, rows: 24 }, { cols: 90, rows: 30 }, 13)
    expect(scaled.font).toBeCloseTo(11.7, 5)
    expect(scaled.pan).toEqual({ cols: 0, rows: 0 })
  })

  it('never clips, at any grid in any pane', () => {
    const sizes = [1, 2, 9, 24, 50, 80, 81, 120, 199, 300, 1000]
    const clipped: string[] = []
    for (const gc of sizes)
      for (const gr of sizes)
        for (const rc of sizes)
          for (const rr of [1, 18, 40, 300]) {
            const f = fitGrid({ cols: gc, rows: gr }, { cols: rc, rows: rr }, 13)
            const holds = rc >= gc && rr >= gr
            const ok =
              f.shown.cols + f.pan.cols === gc &&
              f.shown.rows + f.pan.rows === gr &&
              f.font >= MIN_FONT_PX &&
              f.font <= 13 &&
              // What is shown at the scaled font fits in the pane.
              f.shown.cols * f.font <= rc * 13 + 1e-6 &&
              (!holds || (f.font === 13 && f.pan.cols === 0 && f.pan.rows === 0))
            if (!ok) clipped.push(`${gc}x${gr} in ${rc}x${rr}: ${JSON.stringify(f)}`)
          }
    expect(clipped).toEqual([])
  })
})
