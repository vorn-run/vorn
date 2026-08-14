import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { SURFACE, TERMINAL_BACKGROUND } from '../src/shared/surface'

const root = join(__dirname, '..')
const read = (p: string): string => readFileSync(join(root, p), 'utf8')

const css = read('src/renderer/global.css')

function token(name: string): string {
  const match = css.match(new RegExp(`--color-${name}:\\s*(#[0-9a-fA-F]{3,8});`))
  if (!match) throw new Error(`--color-${name} is not defined in global.css`)
  return match[1].toLowerCase()
}

/** Leading channel of a rung, as a stand-in for how light it reads. */
const level = (hex: string): number => parseInt(hex.slice(1, 3), 16)

const RUNGS = ['base', 'panel', 'sunken', 'overlay'] as const

describe('the surface ladder', () => {
  it('keeps every value that lives outside the stylesheet on its rung', () => {
    // global.css is authoritative. These are the copies that cannot read a
    // custom property, so they are the ones that can silently fall behind it.
    expect(RUNGS.map((r) => [r, SURFACE[r]])).toEqual(RUNGS.map((r) => [r, token(`surface-${r}`)]))
    expect(TERMINAL_BACKGROUND).toBe(token('surface-sunken'))
  })

  it('paints both pre-mount grounds on the field', () => {
    // Before the app mounts there is no stylesheet, so these two spell the
    // field out. Anything above it flashes light on launch and bands light
    // while the window is resized.
    const html = read('src/renderer/index.html')
    const offline = read('packages/web/public/offline.html')

    expect(html).toContain(`background: ${SURFACE.base}`)
    expect(offline).toContain(`background: ${SURFACE.base};`)
  })

  it('steps in small, even increments from the field up to floating chrome', () => {
    // Depth is a step plus a hairline. Larger jumps read as a hole cut in the
    // page rather than a region of it, so the rungs stay close together and in
    // order — that ordering is what lets chrome sit below the work.
    const levels = RUNGS.map((r) => level(token(`surface-${r}`)))
    const gaps = RUNGS.slice(1).map((rung, i) => ({
      step: `${RUNGS[i]} → ${rung}`,
      gap: levels[i + 1] - levels[i]
    }))

    expect(gaps.filter(({ gap }) => gap <= 0 || gap > 10)).toEqual([])
  })

  it('keeps each rung neutral, so no surface carries a hue of its own', () => {
    // Reading one channel would let a rung drift green or blue without the
    // ordering above noticing. Colour belongs to the accent and to the work,
    // never to the ground under them.
    const tinted = RUNGS.filter((rung) => {
      const hex = token(`surface-${rung}`)
      const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16))
      return Math.max(r, g, b) - Math.min(r, g, b) > 4
    })

    expect(tinted).toEqual([])
  })
})
