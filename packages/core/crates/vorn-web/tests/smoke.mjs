// Loads the browser client against a test vornd in a headless Chromium, once
// per renderer: it signs in, opens a terminal, types, and waits for the echo.
// `pair` pairs a fresh browser with a code instead, as a phone does.
// `templates` opens the example extension's settings page and works its
// choice, secret field, Sign in button and more menu, then Settings ›
// Extensions and the Connectors built-in's Connections page. `adapters`
// links a test agent's adapter and the example card footer, starts a session
// of that agent, posts a hook event as the agent would, and waits for the
// footer to show it.
//   node crates/vorn-web/tests/smoke.mjs [webgpu] [webgl] [pair] [templates] [adapters]
// VORN_BIN (default target/ci) holds vornd and vorn-sessiond; WEB_DIST is the
// build-web.sh output; SHOTS, when set, is where screenshots go.
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { request } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'

const core = new URL('../../../', import.meta.url).pathname
const bin = process.env.VORN_BIN ?? join(core, 'target/ci')
const dist = process.env.WEB_DIST ?? join(core, 'crates/vorn-web/dist')
const shots = process.env.SHOTS
const runs = process.argv.slice(2).length
  ? process.argv.slice(2)
  : ['webgpu', 'webgl', 'pair', 'templates', 'adapters']
const turns = join(core, '../../examples/extensions/turns')
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
  // An agent's own home would be the person's, not the test's.
  for (const name of ['COPILOT_HOME', 'XDG_DATA_HOME', 'HERMES_HOME', 'VORN_DATA_DIR'])
    delete env[name]
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

/** Every accessible name and text the page mirrors, for what a template drew. */
const mirrored = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll('[aria-label]')]
      .map((e) => `${e.getAttribute('role') ?? ''}:${e.getAttribute('aria-label')}`)
      .join('\n')
  )

/** Clicks the mirrored element `selector` once it has a box. */
async function press(page, selector, what) {
  const box = await until(what, async () => {
    const b = await page
      .locator(selector)
      .last()
      .boundingBox()
      .catch(() => null)
    return b && b.width > 0 ? b : null
  })
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
}

/** The example extension's settings page, drawn from its template in the browser. */
async function templates(page) {
  await press(page, '[role="button"][aria-label="Settings"]', 'the Settings button')
  await press(page, '[role="button"][aria-label="Turns"]', "the extension's settings page")
  for (const shown of ['radio:Team plan', 'textbox:Paste a key', 'button:Sign in', 'button:More']) {
    await until(shown, async () => (await mirrored(page)).split('\n').includes(shown))
  }
  await press(page, '[role="textbox"][aria-label="Paste a key"]', 'the secret field')
  await page.keyboard.type('hunter2')
  const field = page.locator('[role="textbox"][aria-label="Paste a key"]').last()
  await until('the key masked', async () => {
    const t = await field.textContent()
    return t.includes('\u2022'.repeat(7)) && !t.includes('hunter2')
  })
  await press(page, '[role="button"][aria-label="More"]', 'the more button')
  await press(page, '[role="menuitem"][aria-label="Forget account"]', "the menu's entry")
  await until('the confirm', async () => (await mirrored(page)).includes('Forget Turns account?'))
  await page.keyboard.press('Escape')
  await extensions(page)
}

/** Settings › Extensions lists Connectors among the built-ins; Connections is its page. */
async function extensions(page) {
  await until(
    'the confirm closed',
    async () => !(await mirrored(page)).includes('Forget Turns account?')
  )
  if (!(await mirrored(page)).split('\n').includes('button:Extensions')) {
    await press(page, '[role="button"][aria-label="Settings"]', 'the Settings button')
  }
  await press(page, '[role="button"][aria-label="Extensions"]', 'the Extensions page')
  for (const shown of ['button:Install\u2026', 'button:About Connectors', 'button:About Turns']) {
    await until(shown, async () => (await mirrored(page)).split('\n').includes(shown))
  }
  if (shots)
    await page
      .waitForTimeout(400)
      .then(() => page.screenshot({ path: join(shots, 'web-extensions.png') }))
  await press(page, '[role="button"][aria-label="Connections"]', 'the Connections page')
  await until('button:Add connection', async () =>
    (await mirrored(page)).split('\n').includes('button:Add connection')
  )
  if (shots)
    await page
      .waitForTimeout(400)
      .then(() => page.screenshot({ path: join(shots, 'web-connections.png') }))
}

/** Posts a hook event to the endpoint vornd registered in its HOME, as an agent's hook does. */
async function postHook(vornd, body) {
  const dir = join(vornd.home, '.vorn')
  const [port, token] = await until('the hook endpoint', () => {
    try {
      return ['port', 'token'].map((f) => readFileSync(join(dir, f), 'utf8').trim())
    } catch {
      return null
    }
  })
  const text = JSON.stringify(body)
  const status = await new Promise((resolve, reject) => {
    const req = request(
      {
        host: '127.0.0.1',
        port: Number(port),
        path: '/hooks',
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }
      },
      (res) => {
        res.resume()
        resolve(res.statusCode)
      }
    )
    req.on('error', reject)
    req.end(text)
  })
  if (status !== 200) throw new Error(`the hook answered ${status}`)
}

const footerExample = join(core, '../../examples/extensions/agent-footer')
const testAgent = join(core, 'crates/vorn-app/tests/fixtures/acme-adapter')

/** A session of a linked test agent under the example card footer; answers how to post its tool call. */
async function adapterSession(vornd, server) {
  await server.call('extension:link', { path: testAgent })
  await server.call('extension:link', { path: footerExample })
  // Its program starts on its first call or event; this call starts it.
  await server.call('extension:call', { id: 'example.agent-footer', action: 'start', args: {} })
  const cfg = await server.call('config:load')
  cfg.agentCommands = { ...cfg.agentCommands, acme: { command: '/bin/sh', args: [] } }
  await server.call('config:save', cfg)
  const agent = { id: 'acme' }
  const session = await server.call('terminal:create', {
    agentType: agent.id,
    projectName: 'smoke',
    projectPath: vornd.home,
    model: 'mini'
  })
  return () =>
    postHook(vornd, {
      hook_event_name: 'PreToolUse',
      session_id: 'smoke-conversation',
      cwd: vornd.home,
      tool_name: 'edit',
      vorn_terminal_id: session.id
    })
}

/** The bottom strip of the card named `name`, as pixels: where its footer draws. */
async function cardFoot(page, name) {
  const card = await until('the card', async () => {
    const b = await page
      .locator(`[role="group"][aria-label="${name}"]`)
      .last()
      .boundingBox()
      .catch(() => null)
    return b && b.width > 0 ? b : null
  })
  const clip = { x: card.x, y: card.y + card.height - 64, width: card.width, height: 64 }
  return page.screenshot({ clip })
}

async function run(what, vornd, server) {
  let postToolCall
  const browser = await chromium.launch({ channel: 'chromium', args: gpuArgs })
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  try {
    if (what === 'pair') {
      await pair(page, vornd, server)
    } else if (what === 'adapters') {
      const device = (await server.call('token:create', { name: 'smoke adapters' })).plaintext
      postToolCall = await adapterSession(vornd, server)
      await page.goto(`http://127.0.0.1:${vornd.port}/app/#token=${device}`)
    } else if (what === 'templates') {
      const device = (await server.call('token:create', { name: 'smoke templates' })).plaintext
      await server.call('extension:link', { path: turns })
      // Its program starts on its first call or event; this call starts it.
      await server.call('extension:call', {
        id: 'example.turns',
        action: 'plan',
        args: { id: 'free' }
      })
      await page.goto(`http://127.0.0.1:${vornd.port}/app/#token=${device}`)
    } else {
      const device = (await server.call('token:create', { name: `smoke ${what}` })).plaintext
      await page.goto(`http://127.0.0.1:${vornd.port}/app/?renderer=${what}#token=${device}`)
    }
    const used = await until('a renderer', () =>
      page.evaluate(() => document.querySelector('canvas')?.dataset.renderer)
    )
    if (what === 'webgpu' || what === 'webgl') {
      if (!used.startsWith(what)) throw new Error(`asked for ${what}, got ${used}`)
    }
    if (what === 'adapters') {
      // Template text is drawn, not mirrored: the footer shows as the card's foot changing.
      await sleep(1500)
      const before = await cardFoot(page, 'smoke')
      await postToolCall()
      await until('the footer', async () => !(await cardFoot(page, 'smoke')).equals(before))
      if (shots) await page.screenshot({ path: join(shots, 'web-adapters.png') })
      console.log(`ok adapters (${used})`)
      return
    }
    if (what === 'templates') {
      await templates(page)
      if (shots) await page.screenshot({ path: join(shots, 'web-templates.png') })
      console.log(`ok templates (${used})`)
      return
    }
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
