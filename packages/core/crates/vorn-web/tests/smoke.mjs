// Loads the browser client against a test vornd in a headless Chromium, once
// per renderer: it signs in, opens a terminal, types, and waits for the echo.
// `pair` pairs a fresh browser with a code instead, as a phone does.
//   node crates/vorn-web/tests/smoke.mjs [webgpu] [webgl] [pair]
// VORN_BIN (default target/ci) holds vornd and vorn-sessiond; WEB_DIST is the
// build-web.sh output; SHOTS, when set, is where screenshots go.
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'

const core = new URL('../../../', import.meta.url).pathname
const bin = process.env.VORN_BIN ?? join(core, 'target/ci')
const dist = process.env.WEB_DIST ?? join(core, 'crates/vorn-web/dist')
const shots = process.env.SHOTS
const runs = process.argv.slice(2).length ? process.argv.slice(2) : ['webgpu', 'webgl', 'pair']
const { chromium } = await import(process.env.PLAYWRIGHT ?? 'playwright-core')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// Headless GPU: Metal on a Mac, SwiftShader (Vulkan for WebGPU) elsewhere.
const gpuArgs =
  process.platform === 'darwin'
    ? ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal']
    : [
        '--enable-unsafe-webgpu',
        '--enable-unsafe-swiftshader',
        '--enable-features=Vulkan',
        '--use-vulkan=swiftshader',
        '--use-webgpu-adapter=swiftshader',
        '--use-angle=swiftshader'
      ]

async function until(what, f, ms = 30000) {
  const end = Date.now() + ms
  for (;;) {
    const v = await f()
    if (v) return v
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`)
    await sleep(250)
  }
}

/** A test vornd in its own HOME and data directory, never the real one. */
async function startVornd() {
  // Short: the session holder's socket path must fit in sun_path.
  const home = mkdtempSync(join(process.platform === 'darwin' ? '/tmp' : tmpdir(), 'vw-'))
  const data = join(home, '.vorn')
  mkdirSync(data)
  writeFileSync(join(home, '.zshrc'), '')
  const env = { ...process.env, HOME: home, VORN_HOME: data, ZDOTDIR: home }
  const args = ['--data-dir', data, '--port', '0', '--host', '127.0.0.1', '--web', dist]
  args.push('--sessiond', join(bin, 'vorn-sessiond'), '--exit-with-stdin')
  const proc = spawn(join(bin, 'vornd'), args, { env, stdio: ['pipe', 'pipe', 'inherit'] })
  const [line] = await once(createInterface({ input: proc.stdout }), 'line')
  const { port } = JSON.parse(line)
  const local = readFileSync(join(data, 'local-token'), 'utf8').trim()
  return {
    home,
    port,
    local,
    async stop() {
      proc.stdin.end()
      await Promise.race([once(proc, 'exit'), sleep(5000)])
      proc.kill()
      rmSync(home, { recursive: true, force: true })
    }
  }
}

/** JSON-RPC on vornd's /ws, signed in with `token`. */
async function rpc(port, token) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
  const pending = new Map()
  let next = 1
  let signedIn
  const ok = new Promise((r) => (signedIn = r))
  ws.onmessage = (e) => {
    if (typeof e.data !== 'string') return
    const m = JSON.parse(e.data)
    if (m.method === 'auth:ok') signedIn()
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m)
      pending.delete(m.id)
    }
  }
  await once(ws, 'open')
  ws.send(
    JSON.stringify({ jsonrpc: '2.0', id: next++, method: 'auth:authenticate', params: { token } })
  )
  await ok
  return {
    async call(method, params) {
      const id = next++
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
      const m = await new Promise((r) => pending.set(id, r))
      if (m.error) throw new Error(`${method}: ${JSON.stringify(m.error)}`)
      return m.result
    },
    close: () => ws.close()
  }
}

/** The text the client mirrors for its terminals, all of them. */
const terminalText = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll('[aria-roledescription="terminal"]')]
      .map((e) => e.textContent)
      .join('\n')
  )

/** Pairs the page with a code, approved from the desktop's side. */
async function pair(page, vornd, server) {
  await page.goto(`http://127.0.0.1:${vornd.port}/app/`)
  const field = page.locator('[role="textbox"][aria-label="Pairing code or device token"]')
  const box = await until('the pairing field', async () => {
    const b = await field.boundingBox().catch(() => null)
    return b && b.width > 0 ? b : null
  })
  const { code } = await server.call('pairing:start')
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.type(code)
  await page.keyboard.press('Enter')
  const [request] = await until('the pairing request', async () => {
    const pending = await server.call('pairing:pending')
    return pending.length ? pending : null
  })
  await server.call('pairing:approve', { requestId: request.requestId })
}

async function run(what, vornd, server) {
  const browser = await chromium.launch({ channel: 'chromium', args: gpuArgs })
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  try {
    if (what === 'pair') {
      await pair(page, vornd, server)
    } else {
      const device = (await server.call('token:create', { name: `smoke ${what}` })).plaintext
      await page.goto(`http://127.0.0.1:${vornd.port}/app/?renderer=${what}#token=${device}`)
    }
    const used = await until('a renderer', () =>
      page.evaluate(() => document.querySelector('canvas')?.dataset.renderer)
    )
    if (what !== 'pair' && !used.startsWith(what)) throw new Error(`asked for ${what}, got ${used}`)
    const shown = await until('the client to show its terminal', async () => {
      const box = await page.locator('[aria-roledescription="terminal"]').first().boundingBox()
      return box && box.width > 0 ? box : null
    })
    await page.mouse.click(shown.x + shown.width / 2, shown.y + shown.height / 2)
    await page.keyboard.type('echo vorn-$((6*7))-web')
    await page.keyboard.press('Enter')
    await until('the echo', async () => (await terminalText(page)).includes('vorn-42-web'), 15000)
    if (shots) await page.screenshot({ path: join(shots, `web-${what}.png`) })
    console.log(`ok ${what} (${used})`)
  } catch (e) {
    if (shots)
      await page.screenshot({ path: join(shots, `web-${what}-failed.png`) }).catch(() => {})
    console.error(`page errors: ${errors.join('\n') || 'none'}`)
    console.error(`terminal text: ${await terminalText(page).catch(() => '?')}`)
    throw e
  } finally {
    await browser.close()
  }
}

const vornd = await startVornd()
let failed = false
try {
  const server = await rpc(vornd.port, vornd.local)
  const cfg = await server.call('config:load')
  cfg.projects = [{ name: 'smoke', path: vornd.home, preferredAgents: [] }]
  cfg.defaults = { ...cfg.defaults, hasSeenOnboarding: 99 }
  await server.call('config:save', cfg)
  await server.call('shell:create', vornd.home)
  for (const r of runs) await run(r, vornd, server)
  server.close()
} catch (e) {
  console.error(e)
  failed = true
} finally {
  await vornd.stop()
}
process.exit(failed ? 1 : 0)
