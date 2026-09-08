// Every raster on the page, as WebP at the widths the page asks for.
// Sources stay in assets/ (the README and the og card still use them);
// the page itself references only the .webp files this writes.
// Usage: node scripts/optimize-images.mjs   (needs cwebp on PATH)
import { execFileSync } from 'node:child_process'
import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

const ASSETS = new URL('../assets/', import.meta.url).pathname
// Widths per subject: the hero fills the container at 1400 max, a plate
// half of it; 2x for retina.
const WIDTHS = {
  'hero.png': [1400, 2400],
  'vitruvian-lines.png': [1200],
  default: [900, 1600]
}
const SKIP = new Set(['favicon.png', 'vorn-mark.png', 'vorn-logo.png', 'og.png', 'vitruvian.jpg'])

function size(file) {
  const out = execFileSync('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', file]).toString()
  const w = Number(/pixelWidth: (\d+)/.exec(out)?.[1])
  const h = Number(/pixelHeight: (\d+)/.exec(out)?.[1])
  return { w, h }
}

const manifest = {}
for (const name of readdirSync(ASSETS)) {
  if (!/\.(png|jpe?g)$/i.test(name) || SKIP.has(name)) continue
  const src = join(ASSETS, name)
  const { w, h } = size(src)
  const base = name.replace(/\.(png|jpe?g)$/i, '')
  const widths = (WIDTHS[name] ?? WIDTHS.default).filter((x) => x <= w)
  if (!widths.length) widths.push(w)
  manifest[name] = { w, h, out: [] }
  for (const width of widths) {
    const out = join(ASSETS, `${base}-${width}.webp`)
    execFileSync('cwebp', ['-quiet', '-q', '82', '-resize', String(width), '0', src, '-o', out])
    manifest[name].out.push({ width, file: `${base}-${width}.webp`, bytes: statSync(out).size })
  }
}
for (const [name, m] of Object.entries(manifest)) {
  console.log(
    `${name} ${m.w}x${m.h} -> ${m.out.map((o) => `${o.file} ${(o.bytes / 1024).toFixed(0)}K`).join(', ')}`
  )
}
