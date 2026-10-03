/**
 * Renderer frame time at 1, 8 and 32 terminals.
 *
 * Builds `bench/renderer/` (the real terminal registry, see `page.ts`) with the
 * project's own Vite, serves it on loopback and drives it in Chromium through
 * Playwright. Each terminal is fed 32 KB/s of an agent's TUI output on the
 * server's 8 ms flush beat while frame intervals and long tasks are recorded.
 *
 * Chromium rather than Electron: Electron is this same engine, and launching the
 * app would measure the rest of the UI as well. The GPU string is kept with the
 * result because a software rasteriser (SwiftShader, in a headless container)
 * and a real GPU give different absolute numbers; compare runs from one machine.
 *
 * Browser selection: `VORN_BENCH_CHROMIUM` (an executable path), else
 * Playwright's own Chromium, else an installed Chrome.
 */
import http from 'node:http'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { AddressInfo } from 'node:net'
import { build } from 'vite'
import { round } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import { transcript } from '../lib/transcripts'
import type { RendererRun } from '../renderer/page'

const ROOT = path.resolve(__dirname, '..', '..')
const TERMINALS = [1, 8, 32]
const DURATION_MS = QUICK ? 1500 : 3000
const ROUNDS = QUICK ? 1 : 4
const BYTES_PER_SECOND_EACH = 32 * 1024

type Playwright = typeof import('playwright-core')

async function loadPlaywright(): Promise<Playwright> {
  try {
    return await import('playwright-core')
  } catch {
    return (await import('playwright' as string)) as Playwright
  }
}

async function bundle(): Promise<string> {
  const outDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bench-renderer-'))
  await build({
    configFile: false,
    logLevel: 'error',
    root: path.join(ROOT, 'bench', 'renderer'),
    base: './',
    resolve: {
      alias: {
        '@vornrun/shared/': `${path.join(ROOT, 'packages/shared/src')}/`,
        '@vornrun/shared': path.join(ROOT, 'packages/shared/src/index.ts')
      }
    },
    build: {
      outDir,
      emptyOutDir: true,
      minify: true,
      sourcemap: false,
      modulePreload: { polyfill: false }
    }
  })
  return outDir
}

function serve(dir: string): Promise<http.Server> {
  const types: Record<string, string> = {
    '.html': 'text/html',
    '.js': 'text/javascript',
    '.css': 'text/css'
  }
  const server = http.createServer((req, res) => {
    const rel = decodeURIComponent((req.url ?? '/').split('?')[0])
    const file = path.join(dir, rel === '/' ? 'index.html' : rel)
    if (!file.startsWith(dir) || !fs.existsSync(file)) {
      res.writeHead(404).end()
      return
    }
    res.writeHead(200, { 'content-type': types[path.extname(file)] ?? 'application/octet-stream' })
    fs.createReadStream(file).pipe(res)
  })
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)))
}

/**
 * Headless Chromium on macOS draws WebGL with SwiftShader, a software
 * rasteriser, which measures the CPU rather than the GPU the app actually uses.
 * So on macOS the browser opens a visible window with Metal, unless
 * `VORN_BENCH_HEADLESS=1` asks otherwise. Elsewhere it stays headless: a Linux
 * CI box or container has no display and no GPU to gain.
 */
async function launch(pw: Playwright): Promise<import('playwright-core').Browser> {
  const headless =
    process.env.VORN_BENCH_HEADLESS === '1' ||
    (process.platform !== 'darwin' && process.env.VORN_BENCH_HEADLESS !== '0')
  const args = [
    '--ignore-gpu-blocklist',
    '--enable-unsafe-swiftshader',
    '--disable-background-timer-throttling',
    '--disable-renderer-backgrounding',
    '--disable-backgrounding-occluded-windows',
    ...(process.platform === 'darwin' ? ['--use-angle=metal'] : [])
  ]
  const options = { headless, args }
  const explicit = process.env.VORN_BENCH_CHROMIUM
  if (explicit) return pw.chromium.launch({ ...options, executablePath: explicit })
  try {
    return await pw.chromium.launch(options)
  } catch {
    return pw.chromium.launch({ ...options, channel: 'chrome' })
  }
}

async function main(): Promise<void> {
  const pw = await loadPlaywright()
  const outDir = await bundle()
  const server = await serve(outDir)
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`
  const browser = await launch(pw)
  const chunks = transcript('spinner').chunks
  const version = browser.version()

  const metrics: Record<string, Metric> = {}
  let gpu = ''
  let webgl = 0
  for (const n of TERMINALS) {
    const runs: RendererRun[] = []
    for (let r = 0; r < ROUNDS; r++) {
      const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
      await page.goto(url)
      await page.waitForFunction(
        () => (window as unknown as { __ready?: boolean }).__ready === true
      )
      const result = await page.evaluate(
        (o) => (window as unknown as { __bench: (o: unknown) => Promise<RendererRun> }).__bench(o),
        { terminals: n, chunks, bytesPerSecondEach: BYTES_PER_SECOND_EACH, durationMs: DURATION_MS }
      )
      runs.push(result)
      gpu = result.gpu
      webgl = Math.max(webgl, result.webglCanvases)
      await page.close()
    }
    // Pooled over rounds rather than a median of rounds: a frame interval is a
    // multiple of the vsync period, so per-round figures move in steps and a
    // median of three of them jumps. Percentiles are left out for the same
    // reason; they are quantised to whole frames.
    const frames = runs.reduce((a, x) => a + x.frames, 0)
    const meanFrame = runs.reduce((a, x) => a + x.frameMeanMs * x.frames, 0) / frames
    const longTask = runs.reduce((a, x) => a + x.longTaskMs, 0) / runs.length
    metrics[`frame.mean.${n}t`] = metric(
      round(meanFrame),
      'ms',
      `mean frame interval, ${n} terminal(s) streaming`
    )
    metrics[`longtask.${n}t`] = metric(
      round(longTask / (DURATION_MS / 1000)),
      'ms/s',
      `main-thread long-task time per second, ${n} terminal(s) streaming`
    )
  }

  if (/swiftshader/i.test(gpu)) {
    console.error(
      `renderer: drawn with a software rasteriser (${gpu}); these are CPU numbers, not GPU ones`
    )
  }
  await browser.close()
  server.close()
  fs.rmSync(outDir, { recursive: true, force: true })
  emit({
    suite: 'renderer',
    metrics,
    info: {
      gpu,
      webglCanvasesSeen: webgl,
      browser: version,
      bytesPerSecondEach: BYTES_PER_SECOND_EACH,
      durationMs: DURATION_MS
    }
  })
  process.exit(0)
}

void main()
