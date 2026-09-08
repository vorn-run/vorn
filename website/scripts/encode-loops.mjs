// Each raw take in release-videos/ becomes a webm, an mp4 and a poster the plate can stand on.
// Usage: node scripts/encode-loops.mjs   (needs ffmpeg and cwebp on PATH)
import { execFileSync } from 'node:child_process'
import { existsSync, statSync } from 'node:fs'
import { join } from 'node:path'

const ASSETS = new URL('../assets/', import.meta.url).pathname
const TAKES = new URL('../release-videos/', import.meta.url).pathname

// Delivery width per plate, and the byte budget a loop may not exceed.
const LOOPS = [
  { name: 'durability', width: 1600, budget: 400 },
  { name: 'workflows', width: 1200, budget: 400 },
  { name: 'connectors', width: 900, budget: 400 },
  { name: 'phone', width: 560, budget: 400 },
  { name: 'panes', width: 900, budget: 400 }
]

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
const over = []
for (const { name, width, budget, crop } of LOOPS) {
  const src = join(TAKES, `${name}.mov`)
  if (!existsSync(src)) {
    rows.push(`${name} — no take yet`)
    continue
  }
  const { w, h } = probe(src)
  // Height rounded to an even number so both encoders accept it.
  const scaled = Math.round(((h / w) * width) / 2) * 2
  const filter = [crop ? `crop=${crop}` : null, `scale=${width}:${scaled}`, 'fps=30']
    .filter(Boolean)
    .join(',')

  const webm = join(ASSETS, `${name}.webm`)
  const mp4 = join(ASSETS, `${name}.mp4`)
  ff([
    '-i',
    src,
    '-vf',
    filter,
    '-an',
    '-c:v',
    'libvpx-vp9',
    '-b:v',
    '0',
    '-crf',
    '34',
    '-row-mt',
    '1',
    webm
  ])
  ff([
    '-i',
    src,
    '-vf',
    filter,
    '-an',
    '-c:v',
    'libx264',
    '-crf',
    '26',
    '-pix_fmt',
    'yuv420p',
    '-movflags',
    '+faststart',
    mp4
  ])

  const frame = join(TAKES, `${name}-poster.png`)
  ff(['-ss', '0', '-i', src, '-frames:v', '1', ...(crop ? ['-vf', `crop=${crop}`] : []), frame])
  const posters = [...new Set([900, width])].filter((x) => x <= w)
  for (const pw of posters) {
    execFileSync('cwebp', [
      '-quiet',
      '-q',
      '82',
      '-resize',
      String(pw),
      '0',
      frame,
      '-o',
      join(ASSETS, `${name}-${pw}.webp`)
    ])
  }

  const heaviest = Math.max(kb(webm), kb(mp4))
  if (heaviest > budget) over.push(`${name} ${heaviest.toFixed(0)}K over the ${budget}K budget`)
  rows.push(
    `${name} ${w}x${h} -> ${width}x${scaled} · webm ${kb(webm).toFixed(0)}K · mp4 ${kb(mp4).toFixed(0)}K · ` +
      `poster ${posters.map((p) => `${p}w ${kb(join(ASSETS, `${name}-${p}.webp`)).toFixed(0)}K`).join(', ')}`
  )
}

for (const row of rows) console.log(row)
if (over.length) {
  console.error(`\n${over.join('\n')}`)
  process.exit(1)
}
