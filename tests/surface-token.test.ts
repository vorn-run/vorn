import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { TERMINAL_BACKGROUND } from '../src/renderer/lib/surface'

const css = readFileSync(join(__dirname, '../src/renderer/global.css'), 'utf8')

function token(name: string): string {
  const match = css.match(new RegExp(`--color-${name}:\\s*(#[0-9a-fA-F]{3,8});`))
  if (!match) throw new Error(`--color-${name} is not defined in global.css`)
  return match[1].toLowerCase()
}

describe('the surface ladder', () => {
  it('keeps the terminal canvas on the same rung as the card holding it', () => {
    // xterm takes literal colours, so this is the one surface spelled twice.
    // When only one of the two moved, the terminal read as a lighter rectangle
    // inside its own card.
    expect(TERMINAL_BACKGROUND).toBe(token('surface-sunken'))
  })

  it('steps in small, even increments from the field up to floating chrome', () => {
    // Depth is a step plus a hairline. Larger jumps read as a hole cut in the
    // page rather than a region of it, so the rungs stay close together and in
    // order — that ordering is what lets chrome sit below the work.
    const level = (name: string): number => parseInt(token(name).slice(1, 3), 16)
    const base = level('surface-base')
    const panel = level('surface-panel')
    const sunken = level('surface-sunken')
    const overlay = level('surface-overlay')

    expect(base).toBeLessThan(panel)
    expect(panel).toBeLessThan(sunken)
    expect(sunken).toBeLessThan(overlay)
    expect(panel - base).toBeLessThanOrEqual(5)
    expect(sunken - panel).toBeLessThanOrEqual(5)
  })
})
