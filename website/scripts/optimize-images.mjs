// Every raster the pages ask for, as WebP at the widths they ask for.
// The pages are the source of truth: a name and width in an src or srcset is
// the whole instruction, so nothing is written that nothing loads. Sources stay
// in assets/ because the README still points at them.
// Usage: node scripts/optimize-images.mjs   (needs cwebp on PATH)
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { ASSETS, QUALITY, toWebp } from './lib.mjs'

/** A screenshot is read; the marginalia are only looked at. */
const DRAWING = /^(davinci|vitruvian|vorn-mark)/

const PAGES = ['../index.html', '../404.html'].map((p) => new URL(p, import.meta.url).pathname)

function size(file) {
  const out = execFileSync('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', file]).toString()
  return {
    w: Number(/pixelWidth: (\d+)/.exec(out)?.[1]),
    h: Number(/pixelHeight: (\d+)/.exec(out)?.[1])
  }
}

function source(name) {
  return ['png', 'jpg', 'jpeg']
    .map((ext) => join(ASSETS, `${name}.${ext}`))
    .find((file) => existsSync(file))
}

const wanted = new Map()
for (const page of PAGES) {
  for (const [, name, width] of readFileSync(page, 'utf8').matchAll(
    /assets\/([a-z0-9-]+?)-(\d+)\.webp/g
  )) {
    if (!wanted.has(name)) wanted.set(name, new Set())
    wanted.get(name).add(Number(width))
  }
}

const missing = []
for (const [name, widths] of wanted) {
  const src = source(name)
  // A page asking for a raster with no source is a still nobody has shot yet.
  if (!src) {
    missing.push(name)
    continue
  }
  const { w, h } = size(src)
  const out = [...widths]
    .sort((a, b) => a - b)
    .filter((width) => width <= w)
    .map((width) => {
      const file = `${name}-${width}.webp`
      toWebp(src, width, join(ASSETS, file), DRAWING.test(name) ? '82' : QUALITY)
      return `${file} ${(statSync(join(ASSETS, file)).size / 1024).toFixed(0)}K`
    })
  console.log(`${name} ${w}x${h} -> ${out.join(', ')}`)
}

if (missing.length) console.log(`no source yet: ${missing.join(', ')}`)
