import os from 'node:os'
import log from './logger'

/**
 * Which browsers may open a socket to this server.
 *
 * Browsers apply neither CORS nor same-origin policy to a WebSocket upgrade, and
 * they let a page on any origin connect to `localhost` — so without this, any
 * website the user visited could drive the server. Browsers always set `Origin`
 * and page script cannot forge it, which is what makes the header worth checking.
 *
 * The rule is same-origin: the `Origin` must match the `Host` the client actually
 * dialled. That is what the web client always produces — `getWebSocketUrl()` in
 * `packages/web/src/env.ts` builds its URL from `location.host` — and it holds for
 * loopback, a LAN address, a tailnet address or anything future without a list to
 * maintain.
 *
 * Same-origin alone has one hole, and it is the classic one: a name the attacker
 * chose, pointed at a private address. `http://vorn.attacker.test:PORT` sends an
 * `Origin` and a `Host` derived from the same URL, so of course they match — with
 * DNS rebinding it can even target loopback, needing only the port. So an origin
 * must additionally be *unrebindable*: an IP literal, `localhost`, or a name the
 * operator named. An IP literal has nothing to re-resolve, and to hold such an
 * origin a page must have been served by whatever answers at that address — which
 * is this server.
 *
 * A non-browser client sends no `Origin` and is allowed through to the credential
 * check, which is the control that actually applies to it. That asymmetry is safe
 * because a browser cannot omit the header on an upgrade: the spec requires it.
 */

/** Names the operator has vouched for — a reverse proxy, a tunnel, a tailnet name. */
let trustedHosts = new Set<string>()

/**
 * Hostnames that are legitimate but not IP literals.
 *
 * Recomputed whenever reachability changes rather than derived from the bind, so a
 * stale entry is a *fallback* rather than a lockout: a tailnet client refused by
 * name still connects by its `100.x` literal.
 */
export function setTrustedOriginHosts(hosts: string[]): void {
  trustedHosts = new Set(
    [...hosts, os.hostname(), `${os.hostname()}.local`]
      .filter((h): h is string => typeof h === 'string' && h.length > 0)
      .map(normaliseHostname)
  )
}

/** Test seam, and the state a fresh process starts in. */
export function resetTrustedOriginHosts(): void {
  trustedHosts = new Set()
}

/** Lowercase and drop a trailing dot, which is legal in a name and not in a list. */
function normaliseHostname(hostname: string): string {
  return hostname.toLowerCase().replace(/\.$/, '')
}

/**
 * `host:port` as the URL parser sees it, or null if the header is unusable.
 *
 * Both sides of the comparison go through the same parser. Normalising one and
 * comparing it against the other raw is an asymmetry a client with header control
 * can exploit — `LOCALHOST:3456` against `localhost:3456`, say.
 */
function normaliseHostHeader(raw: string | undefined): string | null {
  if (!raw) return null
  try {
    return new URL(`http://${raw}`).host.toLowerCase()
  } catch {
    return null
  }
}

function isIpLiteral(hostname: string): boolean {
  // The parser has already canonicalised these by the time we see them: IPv4
  // shorthands like `127.1` arrive as `127.0.0.1`, and IPv6 keeps its brackets.
  //
  // This is a shape test, not a validity test, and it does not need to be: a
  // hostname whose last label is numeric is parsed as IPv4, and `new URL` rejects
  // it outright when an octet is out of range. So `999.999.999.999` never arrives
  // here at all, and a numeric-looking name that is not an address cannot reach
  // this and be mistaken for an unrebindable one.
  if (hostname.startsWith('[') && hostname.endsWith(']')) return true
  return /^\d{1,3}(?:\.\d{1,3}){3}$/.test(hostname)
}

/**
 * Whether an upgrade carrying this `Origin` may proceed.
 *
 * Returns true for an absent header — see the note above. Every other path must
 * fail closed.
 */
export function isAllowedUpgrade(
  origin: string | undefined,
  hostHeader: string | undefined
): boolean {
  if (origin === undefined) return true

  let url: URL
  try {
    url = new URL(origin)
  } catch {
    // Node hands us the literal string 'null' for `Origin: null`, which a
    // sandboxed iframe, a `file://` page or a cross-origin redirect can produce —
    // so the absent-check above does not catch it and this must refuse rather
    // than treat an unparseable value as "no origin".
    //
    // This also disposes of duplicate headers: `origin` is not one of the
    // single-value headers Node de-duplicates, so two arrive joined as "a, b",
    // which fails to parse. Never split this value on commas.
    return false
  }

  // One line that rejects userinfo, a path, a trailing slash, an uppercase
  // scheme, a spelled-out default port and non-canonical IPv6. Without it,
  // `http://evil.example@127.0.0.1:3456` has a `host` of `127.0.0.1:3456` and
  // would pass a bare host comparison.
  if (url.origin !== origin) return false

  // Deliberately not compared against the request's scheme: behind a
  // TLS-terminating proxy the server cannot know its own.
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false

  const host = normaliseHostHeader(hostHeader)
  if (host === null || url.host.toLowerCase() !== host) return false

  const hostname = normaliseHostname(url.hostname)
  // `localhost` is not rebindable in practice: RFC 6761 forbids resolvers from
  // sending it to DNS, and every engine maps it to loopback internally.
  return isIpLiteral(hostname) || hostname === 'localhost' || trustedHosts.has(hostname)
}

/** Log once at the point of refusal, so a 403 is not silent in the server log. */
export function logRefusedUpgrade(
  origin: string | undefined,
  hostHeader: string | undefined
): void {
  log.warn({ origin, host: hostHeader }, '[ws] refused upgrade from disallowed origin')
}
