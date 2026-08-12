/**
 * URL handling for session browser panes.
 *
 * People type `localhost:5173`, `example.com`, or a bare path far more often
 * than they type a full absolute URL, so the address bar has to guess. Getting
 * that guess wrong is worse than useless: an unrecognised scheme handed to a
 * `<webview>` either fails silently or, for `file://` and `javascript:`, hands
 * the page more reach than a session pane should have.
 */

/** Schemes a session pane is allowed to load. */
const ALLOWED_PROTOCOLS = new Set(['http:', 'https:', 'about:'])

/**
 * Turn whatever the user typed into a loadable URL, or null if it can't be one.
 *
 * Bare hosts and `host:port` get `http://` when they look local and `https://`
 * otherwise — dev servers are the common case for a coding tool, and they rarely
 * speak TLS.
 */
export function normalizeUrl(input: string): string | null {
  const raw = input.trim()
  if (!raw) return null

  // `localhost:5173` and `myapp.local:3000` are valid URLs whose *scheme* is
  // `localhost:` / `myapp.local:`. They must be recognised as host:port before
  // the scheme check below, or the most common input a dev types is rejected.
  if (/^[a-z0-9.-]+:\d+(\/.*)?$/i.test(raw)) return buildUrl(raw)

  // Already absolute: accept only schemes we're willing to render.
  if (/^[a-z][a-z0-9+.-]*:/i.test(raw)) {
    try {
      const parsed = new URL(raw)
      return ALLOWED_PROTOCOLS.has(parsed.protocol) ? parsed.toString() : null
    } catch {
      return null
    }
  }

  if (raw.startsWith('//')) return buildUrl(raw.slice(2))
  return buildUrl(raw)
}

function buildUrl(hostAndPath: string): string | null {
  const scheme = isLocalHost(hostAndPath) ? 'http://' : 'https://'
  try {
    const parsed = new URL(scheme + hostAndPath)
    if (!parsed.hostname) return null
    return parsed.toString()
  } catch {
    return null
  }
}

/** Localhost, loopback, and `.local` — the things a dev server listens on. */
function isLocalHost(value: string): boolean {
  const host = value.split('/')[0].split(':')[0].toLowerCase()
  return (
    host === 'localhost' ||
    host === '127.0.0.1' ||
    host === '0.0.0.0' ||
    host === '[::1]' ||
    host.endsWith('.local') ||
    host.endsWith('.localhost')
  )
}

/**
 * Compact label for a URL — the host, plus a port when it's the distinguishing
 * part. Used in pane headers and dock pills where the full URL never fits.
 */
export function displayHost(url: string): string {
  try {
    const parsed = new URL(url)
    // `about:blank` and friends parse cleanly but have no hostname; showing an
    // empty header is worse than showing the url itself.
    if (!parsed.hostname) return url
    return parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname
  } catch {
    return url
  }
}
