// Polyfill crypto.randomUUID for non-secure contexts (plain HTTP over Tailscale)
if (typeof crypto !== 'undefined' && !crypto.randomUUID) {
  crypto.randomUUID = () => {
    const bytes = crypto.getRandomValues(new Uint8Array(16))
    bytes[6] = (bytes[6] & 0x0f) | 0x40 // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80 // variant 1
    const h = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
    return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}` as `${string}-${string}-${string}-${string}-${string}`
  }
}

import { registerSW } from 'virtual:pwa-register'
import { createApiShim, AuthRequiredError, storeToken } from './api-shim'
import { getWebSocketUrl } from './env'
import { renderTokenPrompt } from './token-prompt'

// Register service worker for PWA installability and asset caching.
// This runs independently of the WebSocket connection.
registerSW({ immediate: true })

// Mount the API shim on window.api BEFORE any React code loads.
// This is critical: stores and components access window.api at import time.
const api = createApiShim(getWebSocketUrl())
;(window as unknown as { api: typeof api }).api = api

// A rejected credential can arrive at any time — on first load, or on a
// reconnect after the token was revoked. Registered before __ready() so the
// first-load case is covered too.
const askForToken = (): void =>
  renderTokenPrompt((token) => {
    storeToken(token)
    location.reload()
  })

api.__onAuthRequired(askForToken)

/**
 * A bundle and a server that disagree about the protocol.
 *
 * Almost always this page: a service worker serving a build cached before the
 * server was updated. Reloading is what fetches the new one, so that is what it
 * asks for. Said out loud because the alternative is failing later in ways that
 * read as the app being broken rather than merely stale.
 */
api.__onVersionMismatch((server, client) => {
  const root = document.getElementById('root')
  if (!root) return
  root.textContent = ''
  root.setAttribute(
    'style',
    'min-height:100vh;display:flex;align-items:center;justify-content:center;text-align:center;' +
      'background:#0d0d0f;color:#faf9f7;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif'
  )
  const card = document.createElement('div')
  card.setAttribute('style', 'max-width:380px;display:flex;flex-direction:column;gap:12px')

  const heading = document.createElement('h1')
  heading.setAttribute('style', 'margin:0;font-size:20px;font-weight:600')
  heading.textContent = server > client ? 'This page is out of date' : 'The server is out of date'

  const body = document.createElement('p')
  body.setAttribute('style', 'margin:0;font-size:13px;line-height:1.6;color:#8a877f')
  body.textContent =
    server > client
      ? 'Vorn has been updated on the server. Reload to get the current version.'
      : 'This page is newer than the Vorn it is talking to. Update Vorn on that machine.'

  card.append(heading, body)

  if (server > client) {
    const button = document.createElement('button')
    button.setAttribute(
      'style',
      'align-self:center;padding:8px 14px;border-radius:4px;border:1px solid rgba(255,255,255,0.14);' +
        'background:transparent;color:#faf9f7;font-size:13px;cursor:pointer'
    )
    button.textContent = 'Reload'
    // `reload(true)` is long gone, and the service worker is what holds the old
    // bundle — so drop its caches first, or the reload serves the same files.
    button.addEventListener('click', () => {
      void caches
        ?.keys()
        .then((keys) => Promise.all(keys.map((k) => caches.delete(k))))
        .catch(() => undefined)
        .then(() => location.reload())
    })
    card.append(button)
  }

  root.append(card)
})

// Wait for WebSocket connection before rendering
api
  .__ready()
  .then(async () => {
    // Dynamic import so React + App only load after shim is ready
    const { createRoot } = await import('react-dom/client')
    const { App } = await import('@renderer/App')

    // Import the global CSS (Tailwind + custom styles)
    await import('./global.css')

    const root = createRoot(document.getElementById('root')!)
    root.render(<App />)
  })
  .catch((err) => {
    // The prompt is rendered by the __onAuthRequired handler above, which also
    // covers the reconnect case this promise cannot see.
    if (err instanceof AuthRequiredError) return
    throw err
  })
