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
