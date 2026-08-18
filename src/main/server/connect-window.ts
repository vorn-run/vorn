import { BrowserWindow, ipcMain, app } from 'electron'
import path from 'node:path'
import log from '../logger'
import { readHostSettings, writeHostSettings, clearHostSettings } from './host-store'

/**
 * The window shown when this desktop is pointed at a server it cannot reach.
 *
 * It exists because the rest of the app cannot render without a bridge: every
 * store reads config over it, and `createWindow` is only reached once one is
 * connected. A host being unreachable is ordinary — a sleeping machine, a changed
 * network — so there has to be something on screen that is not the app, and that
 * can put a mistyped address right.
 *
 * Self-contained markup rather than the renderer bundle, for the same reason: the
 * bundle needs the server this window exists because we could not reach.
 */
let connectWindow: BrowserWindow | null = null

/** Applying either choice re-runs startup from the top, which is where the
 *  spawn-or-connect decision lives. Cheaper than teaching every subsystem to be
 *  re-initialisable, and it happens at most once per attempt. */
function restart(): void {
  // Deferred by a tick so the IPC handler that called this can return first.
  // `app.exit` is immediate, so restarting inline killed the process before the
  // reply was flushed: the connect window never minds, but Settings awaits that
  // reply, and a caller waiting on a promise that can never settle is a hang with
  // nothing on screen to explain it.
  setImmediate(() => {
    app.relaunch()
    app.exit(0)
  })
}

export function registerConnectHandlers(): void {
  // Registered before `launchServer`, unlike every other handler, because these
  // are the only ones that must work without a bridge.
  ipcMain.handle('connect:get', () => {
    const { mode, url, token } = readHostSettings()
    // The token never goes back to a renderer. Whether one is stored is all the
    // window needs in order to say "reconnecting" rather than "enter a token".
    return { mode, url, hasToken: Boolean(token) }
  })

  ipcMain.handle('connect:save', (_event, params: { url: string; token: string }) => {
    const url = params.url.trim()
    const token = params.token.trim()
    if (!url || !token) return { ok: false, error: 'Both an address and a token are needed.' }
    writeHostSettings({ mode: 'host', url: normaliseHostUrl(url), token })
    restart()
    return { ok: true }
  })

  ipcMain.handle('connect:useLocal', () => {
    clearHostSettings()
    restart()
    return { ok: true }
  })
}

/**
 * Accept what a person would paste.
 *
 * The address they have is the one from Settings — `http://host:port/app/` — and
 * the bridge needs `ws://host:port/ws`. Making them convert it by hand is a step
 * that exists only because of an implementation detail.
 */
export function normaliseHostUrl(input: string): string {
  let value = input.trim()
  if (!/^[a-z]+:\/\//i.test(value)) value = `http://${value}`

  let parsed: URL
  try {
    parsed = new URL(value)
  } catch {
    return input.trim()
  }

  const secure = parsed.protocol === 'https:' || parsed.protocol === 'wss:'
  return `${secure ? 'wss:' : 'ws:'}//${parsed.host}/ws`
}

export function showConnectWindow(reason: string): void {
  if (connectWindow && !connectWindow.isDestroyed()) {
    connectWindow.focus()
    return
  }

  connectWindow = new BrowserWindow({
    width: 520,
    height: 460,
    resizable: false,
    show: false,
    backgroundColor: '#0e0e10',
    title: 'Connect to Vorn',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  })

  connectWindow.once('ready-to-show', () => connectWindow?.show())
  connectWindow.on('closed', () => {
    connectWindow = null
    // Closing this window with nothing connected means there is no app behind it.
    if (BrowserWindow.getAllWindows().length === 0) app.quit()
  })

  log.info('[connect] showing the connect window')
  void connectWindow.loadURL(
    `data:text/html;charset=utf-8,${encodeURIComponent(connectMarkup(reason))}`
  )
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function connectMarkup(reason: string): string {
  return `<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<style>
  :root { color-scheme: dark }
  * { box-sizing: border-box }
  body {
    margin: 0; padding: 28px; background: #0e0e10; color: #f4f2ee;
    font: 13px/1.6 -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }
  h1 { font-size: 17px; margin: 0 0 6px; font-weight: 600 }
  p { margin: 0 0 16px; color: #97938a; font-size: 12px }
  .reason {
    background: #17171a; border: 1px solid #26262b; border-radius: 4px;
    padding: 8px 10px; margin-bottom: 16px; color: #c9922a;
    font: 11px ui-monospace, SFMono-Regular, Menlo, monospace; word-break: break-all;
  }
  label { display: block; font-size: 11px; color: #97938a; margin: 0 0 4px }
  input {
    width: 100%; background: #17171a; border: 1px solid #26262b; border-radius: 4px;
    padding: 8px 10px; color: #f4f2ee; font-size: 13px; margin-bottom: 14px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  input:focus { outline: none; border-color: #4a4a52 }
  button {
    border-radius: 4px; padding: 7px 12px; font-size: 12px; font-weight: 500;
    cursor: pointer; background: transparent; color: #f4f2ee;
    border: 1px solid #35353c; margin-right: 8px;
  }
  button:hover { background: #202024 }
  button.quiet { border-color: transparent; color: #97938a }
  .note { font-size: 11px; color: #6e6b64; margin-top: 18px; border-top: 1px solid #26262b; padding-top: 14px }
  .err { color: #c9922a; font-size: 11px; min-height: 15px; margin-bottom: 6px }
</style></head>
<body>
  <h1>Cannot reach that Vorn</h1>
  <p>The server this app is pointed at did not answer.</p>
  <div class="reason">${escapeHtml(reason)}</div>

  <label for="url">Server address</label>
  <input id="url" placeholder="192.168.0.4:61601" spellcheck="false" autocomplete="off">

  <label for="token">Device token</label>
  <input id="token" type="password" placeholder="vorn_..." spellcheck="false" autocomplete="off">

  <div class="err" id="err"></div>
  <button id="connect">Connect</button>
  <button id="local" class="quiet">Run a server on this machine</button>

  <div class="note">
    A host runs your sessions and holds your data. Workflows and schedules still
    need a desktop attached to the host to execute.
  </div>

<script>
  const $ = (id) => document.getElementById(id)
  window.api.getConnectSettings().then((s) => { if (s && s.url) $('url').value = s.url })

  async function connect() {
    $('err').textContent = ''
    const result = await window.api.saveConnectSettings({
      url: $('url').value,
      token: $('token').value
    })
    if (!result.ok) $('err').textContent = result.error
  }

  $('connect').addEventListener('click', connect)
  $('local').addEventListener('click', () => window.api.useLocalServer())
  document.addEventListener('keydown', (e) => { if (e.key === 'Enter') connect() })
</script>
</body></html>`
}
