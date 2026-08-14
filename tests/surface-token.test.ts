import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { SURFACE, TERMINAL_BACKGROUND } from '../src/shared/surface'

const root = join(__dirname, '..')
const read = (p: string): string => readFileSync(join(root, p), 'utf8')

const css = read('src/renderer/theme.css')

/** Every stylesheet that is the sole entry point for one of the clients. */
const ENTRY_SHEETS = ['src/renderer/global.css', 'packages/web/src/global.css'] as const

/** Every ground painted before a client's stylesheet has loaded. */
const PRE_MOUNT_GROUNDS = [
  'src/renderer/index.html',
  'packages/web/index.html',
  'packages/web/public/offline.html'
] as const

function token(name: string): string {
  const match = css.match(new RegExp(`--color-${name}:\\s*(#[0-9a-fA-F]{3,8});`))
  if (!match) throw new Error(`--color-${name} is not defined in theme.css`)
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

  it('paints every pre-mount ground on the field', () => {
    // Before a client's stylesheet loads there is nothing to read a token from,
    // so these spell the field out. Anything above it flashes light on launch
    // and bands light while the window is resized.
    const wrong = PRE_MOUNT_GROUNDS.filter((p) => !read(p).includes(`background: ${SURFACE.base}`))
    expect(wrong).toEqual([])
  })

  it('gives every client the palette, not just the one that owns it', () => {
    // Both clients mount the same renderer, but each has its own Tailwind entry
    // and only one of them defined the tokens. Utilities silently stopped being
    // generated for the other, and an inline var() that resolves to nothing is
    // an invalid background — so every tokenised surface went transparent on
    // the web client the moment this pass replaced its literals.
    const missing = ENTRY_SHEETS.filter((p) => !/@import\s+['"][^'"]*theme\.css['"]/.test(read(p)))
    expect(missing).toEqual([])

    // And exactly one file may declare them, or the two drift.
    const declaring = [...ENTRY_SHEETS, 'src/renderer/theme.css'].filter((p) =>
      read(p).includes('--color-surface-base:')
    )
    expect(declaring).toEqual(['src/renderer/theme.css'])
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
