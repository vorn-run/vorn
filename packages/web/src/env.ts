import { PHONE_BASE_TOPICS, topicsQuery } from '@vornrun/shared/topics'

/**
 * Detect the WebSocket URL for connecting to the Vorn server.
 *
 * In development (Vite dev server), the proxy handles /ws → server.
 * In production (served by Fastify at /app/), connect to the same host.
 */
export function getWebSocketUrl(): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
  // Declared on the upgrade, so no PTY byte is sent before the filter is known.
  return `${protocol}//${location.host}/ws?${topicsQuery(PHONE_BASE_TOPICS)}`
}

/** vornd's grid-mode WebSocket: the screen as cells rather than bytes. */
export function getGridUrl(): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${location.host}/grid`
}

const GRID_STORAGE_KEY = 'vorn.gridMode'

/**
 * Whether grid mode is on: `?grid=1` on the page, or `vorn.gridMode` set to
 * `1` in local storage. Off by default; terminals keep the bytes path.
 */
export function isGridEnabled(): boolean {
  try {
    if (new URLSearchParams(location.search).get('grid') === '1') return true
    return localStorage.getItem(GRID_STORAGE_KEY) === '1'
  } catch {
    return false
  }
}
