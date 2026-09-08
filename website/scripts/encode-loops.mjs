// Each take in release-videos/ becomes the files its <video> asks for, plus the
// poster it stands on. index.html is the source of truth: the poster name gives
// the take, and data-ladder says this one is worth real bitrate.
//
// Two jobs, two settings. A plate loop is a tight crop — little detail per
// frame, so constant quality is cheap and looks right. The hero is the whole
// canvas, which is the one shot compression ruins, so it gets a bitrate ladder
// and the page picks a rung.
//
// Usage: node scripts/encode-loops.mjs   (needs ffmpeg and cwebp on PATH)
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { toWebp } from './lib.mjs'

const TAKES = new URL('../release-videos/', import.meta.url).pathname
const LOOPS = new URL('../assets/loops/', import.meta.url).pathname
const PAGES = ['../index.html', '../demos.html'].map((p) => new URL(p, import.meta.url).pathname)

const PLATE_BUDGET_KB = 900
const HERO_BUDGET_KB = 1800
/** 6 Mbps at 1600 wide, scaled by area for the smaller rungs. */
const BITRATE_AT_1600 = 6000

const attr = (tag, name) => new RegExp(`${name}="([^"]*)"`).exec(tag)?.[1]

function videosInMarkup() {
  const html = PAGES.map((p) => readFileSync(p, 'utf8')).join('\n')
  return [...html.matchAll(/<video\b[^>]*>/g)]
    .map((m) => m[0])
    .map((tag) => {
      const poster = attr(tag, 'poster') ?? ''
      const named = /assets\/loops\/([a-z0-9-]+)-(\d+)\.webp/.exec(poster)
      if (!named) return null
      const ladder = attr(tag, 'data-ladder')
      return {
        name: named[1],
        posterWidth: Number(named[2]),
        height: Number(attr(tag, 'height')),
        width: Number(attr(tag, 'width')),
        widths: ladder ? ladder.split(',').map(Number) : [Number(attr(tag, 'width'))]
      }
    })
    .filter(Boolean)
}

const ff = (args) => execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', ...args])
const kb = (file) => statSync(file).size / 1024

function probe(file) {
  const out = execFileSync('ffprobe', [
    '-v',
    'error',
    '-select_streams',
    'v:0',
    '-show_entries',
    'stream=width,height',
    '-of',
    'csv=p=0',
    file
  ])
    .toString()
    .trim()
  const [w, h] = out.split(',').map(Number)
  return { w, h }
}

mkdirSync(LOOPS, { recursive: true })
const rows = []
const wrong = []

for (const loop of videosInMarkup()) {
  const { name, widths, width, height, posterWidth } = loop
  const src = join(TAKES, `${name}.mov`)
  if (!existsSync(src)) {
    rows.push(`${name} — no take yet`)
    continue
  }
  const { w, h } = probe(src)
  const isLadder = widths.length > 1
  const even = (n) => Math.round(n / 2) * 2
  const scaledAtWidth = (target) => even((h / w) * target)

  if (scaledAtWidth(width) !== height) {
    wrong.push(
      `${name} encodes to ${width}x${scaledAtWidth(width)}, but its frame reserves ${width}x${height}`
    )
  }

  const made = []
  for (const target of widths) {
    const out = join(LOOPS, isLadder ? `${name}-${target}.mp4` : `${name}.mp4`)
    const filter = `scale=${target}:${scaledAtWidth(target)},fps=${isLadder ? 30 : 15}`
    if (isLadder) {
      // Bitrate scales with area, so every rung looks the same at its own size.
      const rate = Math.round(BITRATE_AT_1600 * (target / 1600) ** 2)
      // prettier-ignore
      ff(['-i', src, '-vf', filter, '-an', '-c:v', 'libx264', '-b:v', `${rate}k`,
          '-maxrate', `${Math.round(rate * 1.2)}k`, '-bufsize', `${rate * 2}k`,
          '-pix_fmt', 'yuv420p', '-movflags', '+faststart', out])
    } else {
      // prettier-ignore
      ff(['-i', src, '-vf', filter, '-an', '-c:v', 'libx264', '-crf', '26',
          '-pix_fmt', 'yuv420p', '-movflags', '+faststart', out])
      const webm = join(LOOPS, `${name}.webm`)
      // prettier-ignore
      ff(['-i', src, '-vf', filter, '-an', '-c:v', 'libvpx-vp9', '-b:v', '0',
          '-crf', '34', '-row-mt', '1', webm])
      made.push(`webm ${kb(webm).toFixed(0)}K`)
    }
    made.push(`${target}w ${kb(out).toFixed(0)}K`)
    const budget = isLadder ? HERO_BUDGET_KB : PLATE_BUDGET_KB
    if (kb(out) > budget) {
      wrong.push(`${name} at ${target}w is ${kb(out).toFixed(0)}K, over the ${budget}K allowed`)
    }
  }

  const frame = join(TAKES, `${name}-poster.png`)
  ff(['-ss', '0', '-i', src, '-frames:v', '1', frame])
  // A poster is fetched whatever preload says, and it only has to hold the eye
  // until the first frame lands, so it is worth less quality than a still.
  const poster = toWebp(frame, posterWidth, join(LOOPS, `${name}-${posterWidth}.webp`), '62')
  rows.push(`${name} ${w}x${h} · ${made.join(' · ')} · poster ${kb(poster).toFixed(0)}K`)
}

for (const row of rows) console.log(row)
if (wrong.length) {
  console.error(`\n${wrong.join('\n')}`)
  process.exit(1)
}
