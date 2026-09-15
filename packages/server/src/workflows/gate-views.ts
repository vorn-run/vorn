import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'

/** Largest review page a gate keeps. */
export const GATE_VIEW_MAX_BYTES = 5 * 1024 * 1024

/** A review page runs its own scripts and styles, loads nothing from the network, and cannot leave its frame. */
export const GATE_VIEW_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data:; form-action 'none'; sandbox allow-scripts"

const segment = (id: string): string => id.replace(/[^A-Za-z0-9._-]/g, '_')

function runDir(dataDir: string, runId: string): string {
  return path.join(dataDir, 'gate-views', segment(runId))
}

export function gateViewFile(
  dataDir: string,
  runId: string,
  nodeId: string,
  round: number
): string {
  return path.join(runDir(dataDir, runId), `${segment(nodeId)}-${round}.html`)
}

const tooBig = (bytes: number): string =>
  `The review page is ${(bytes / 1024 / 1024).toFixed(1)} MB; the limit is 5 MB.`

/** Keep this round's review page beside the run: the token that unlocks it, or why there is none. */
export function publishGateView(
  dataDir: string,
  runId: string,
  nodeId: string,
  round: number,
  view: string
): { token: string } | { error: string } {
  const text = view.trim()
  if (!text) return { error: 'The review page came out empty.' }
  try {
    let html = text
    if (!text.startsWith('<')) {
      if (!/\.html?$/i.test(text)) {
        return { error: `The review page is neither HTML nor a .html file: ${text.slice(0, 120)}` }
      }
      if (!fs.existsSync(text)) return { error: `The review page file does not exist: ${text}` }
      const size = fs.statSync(text).size
      if (size > GATE_VIEW_MAX_BYTES) return { error: tooBig(size) }
      html = fs.readFileSync(text, 'utf8')
    }
    const bytes = Buffer.byteLength(html)
    if (bytes > GATE_VIEW_MAX_BYTES) return { error: tooBig(bytes) }
    const file = gateViewFile(dataDir, runId, nodeId, round)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, html)
    return { token: crypto.randomBytes(16).toString('hex') }
  } catch (err) {
    return { error: `The review page could not be kept: ${(err as Error).message}` }
  }
}

export function sameToken(expected: string, given: string): boolean {
  const a = Buffer.from(expected)
  const b = Buffer.from(given)
  return a.length === b.length && crypto.timingSafeEqual(a, b)
}

export function removeGateViews(dataDir: string, runId: string): void {
  fs.rmSync(runDir(dataDir, runId), { recursive: true, force: true })
}

/** Drop the review pages of runs no longer kept. */
export function sweepGateViews(dataDir: string, keptRunIds: Iterable<string>): void {
  let dirs: string[]
  try {
    dirs = fs.readdirSync(path.join(dataDir, 'gate-views'))
  } catch {
    return
  }
  const kept = new Set([...keptRunIds].map(segment))
  for (const dir of dirs) {
    if (!kept.has(dir))
      fs.rmSync(path.join(dataDir, 'gate-views', dir), { recursive: true, force: true })
  }
}
