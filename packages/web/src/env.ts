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
