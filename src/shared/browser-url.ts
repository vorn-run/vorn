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
 * Scheme names that must never be read as a bare host.
 *
 * `javascript:1` is indistinguishable from `myhost:1` by shape alone, so the
 * host:port fast path below would happily turn it into `https://javascript:1/`
 * — a refusal that silently became an allow. These names are not registrable
 * hostnames, so excluding them costs nothing real.
 */
const SCHEME_NAMES = new Set(['javascript', 'file', 'data', 'blob', 'vbscript', 'about', 'chrome'])

/**
 * How far a caller is allowed to reach.
 *
 * `allowFile` exists because the invariant this module states — the schemes a
 * person cannot type are the ones an agent cannot reach — is deliberately
 * broken in exactly one place: a session whose pane is scoped to a project
 * directory may open files inside it. Breaking it takes an explicit argument
 * rather than a quiet edit to the allowlist, and it is off unless asked for, so
 * the address bar and every existing caller keep the old behaviour.
 *
 * This says nothing about *which* files. A `file:` url that passes here has
 * only been judged well-formed; whether it is inside the session's root is a
 * filesystem question, answered in main where the filesystem is.
 */
export interface UrlOptions {
  allowFile?: boolean
}

/**
 * Turn whatever the user typed into a loadable URL, or null if it can't be one.
 *
 * Bare hosts and `host:port` get `http://` when they look local and `https://`
 * otherwise — dev servers are the common case for a coding tool, and they rarely
 * speak TLS.
 */
export function normalizeUrl(input: string, opts: UrlOptions = {}): string | null {
  const raw = input.trim()
  if (!raw) return null

  // `localhost:5173` and `myapp.local:3000` are valid URLs whose *scheme* is
  // `localhost:` / `myapp.local:`. They must be recognised as host:port before
  // the scheme check below, or the most common input a dev types is rejected.
  if (/^[a-z0-9.-]+:\d+(\/.*)?$/i.test(raw)) {
    const name = raw.slice(0, raw.indexOf(':')).toLowerCase()
    if (!SCHEME_NAMES.has(name)) return buildUrl(raw)
    return null
  }

  // Already absolute: accept only schemes we're willing to render.
  if (/^[a-z][a-z0-9+.-]*:/i.test(raw)) {
    try {
      const parsed = new URL(raw)
      // `about:` is allowed only as `about:blank`. The scheme as a whole
      // covers `about:srcdoc`, `about:cache` and friends, which are browser
      // internals rather than pages anyone meant to open.
      if (parsed.protocol === 'about:') return raw === 'about:blank' ? 'about:blank' : null
      // A file url only ever survives when the caller asked for it. `hostname`
      // must be empty: `file://evil.com/etc/passwd` is a UNC path to another
      // machine, not a local file, and it would be read as one.
      if (parsed.protocol === 'file:') {
        if (!opts.allowFile || parsed.hostname) return null
        // Query and fragment cannot address a file, and carrying them would
        // hand the containment check below a path that is not the path read.
        parsed.search = ''
        parsed.hash = ''
        return parsed.toString()
      }
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
  const authority = value.split('/')[0]
  // IPv6 literals are bracketed and full of colons, so the port can only be
  // split off after the closing bracket — `[::1]:5173` must not become `[`.
  const host = authority.startsWith('[')
    ? authority.slice(0, authority.indexOf(']') + 1).toLowerCase()
    : authority.split(':')[0].toLowerCase()
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
  // A blank page has no host, and "about:blank" is jargon. Every browser calls
  // that tab "New tab", which reads as a place to type rather than as a page
  // that failed to load.
  if (url === 'about:blank') return 'New tab'
  try {
    const parsed = new URL(url)
    // A local file has no host, and its path is the whole of it — a tab is a
    // few characters wide, so the leading directories push the only part that
    // identifies the file off the end. The filename is what a person calls it.
    if (parsed.protocol === 'file:') {
      const name = parsed.pathname.split('/').filter(Boolean).pop()
      return name ? decodeURIComponent(name) : url
    }
    // Other schemes without a hostname parse cleanly but have nothing to show;
    // an empty header is worse than showing the url itself.
    if (!parsed.hostname) return url
    return parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname
  } catch {
    return url
  }
}
