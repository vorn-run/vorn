// Each take in release-videos/ becomes a webm, an mp4 and the poster its plate stands on.
// The plate markup in index.html says which loops exist and how wide they ship, so the
// two cannot drift: a poster this writes is the one the page already asks for.
// Usage: node scripts/encode-loops.mjs   (needs ffmpeg and cwebp on PATH)
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { toWebp } from './lib.mjs'

const TAKES = new URL('../release-videos/', import.meta.url).pathname
const LOOPS = new URL('../assets/loops/', import.meta.url).pathname
const INDEX = new URL('../index.html', import.meta.url).pathname
const BUDGET_KB = 400

const loops = [
  ...readFileSync(INDEX, 'utf8').matchAll(
    /poster="assets\/loops\/([a-z-]+)-(\d+)\.webp"\s+width="(\d+)"\s+height="(\d+)"/g
  )
].map(([, name, width, , height]) => ({ name, width: Number(width), height: Number(height) }))

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

const rows = []
const wrong = []
for (const { name, width, height } of loops) {
  const src = join(TAKES, `${name}.mov`)
  if (!existsSync(src)) {
    rows.push(`${name} — no take yet`)
    continue
  }
  const { w, h } = probe(src)
  // Height rounded to an even number, which both encoders require.
  const scaled = Math.round(((h / w) * width) / 2) * 2
  if (scaled !== height) {
    wrong.push(`${name} encodes to ${width}x${scaled}, but the plate reserves ${width}x${height}`)
  }
  const filter = `scale=${width}:${scaled},fps=30`

  const webm = join(LOOPS, `${name}.webm`)
  const mp4 = join(LOOPS, `${name}.mp4`)
  // prettier-ignore
  ff(['-i', src, '-vf', filter, '-an', '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '34', '-row-mt', '1', webm])
  // prettier-ignore
  ff(['-i', src, '-vf', filter, '-an', '-c:v', 'libx264', '-crf', '26', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', mp4])

  const frame = join(TAKES, `${name}-poster.png`)
  ff(['-ss', '0', '-i', src, '-frames:v', '1', frame])
  const poster = toWebp(frame, width, join(LOOPS, `${name}-${width}.webp`))

  const heaviest = Math.max(kb(webm), kb(mp4))
  if (heaviest > BUDGET_KB) {
    wrong.push(`${name} is ${heaviest.toFixed(0)}K, over the ${BUDGET_KB}K a loop may weigh`)
  }
  rows.push(
    `${name} ${w}x${h} -> ${width}x${scaled} · webm ${kb(webm).toFixed(0)}K · ` +
      `mp4 ${kb(mp4).toFixed(0)}K · poster ${kb(poster).toFixed(0)}K`
  )
}

for (const row of rows) console.log(row)
if (wrong.length) {
  console.error(`\n${wrong.join('\n')}`)
  process.exit(1)
}
