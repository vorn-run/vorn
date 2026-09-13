/** An origin a connector may act on: `https://host`, or `https://*.host` for every subdomain. */
export const ORIGIN_PATTERN = /^https:\/\/(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)+$/i

/** Whether `url` is on one of the declared origins. */
export function withinOrigins(origins: readonly string[], url: string): boolean {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return false
  }
  if (parsed.protocol !== 'https:' || parsed.port !== '') return false
  const target = parsed.hostname.toLowerCase()
  return origins.some((origin) => {
    if (!ORIGIN_PATTERN.test(origin)) return false
    const wildcard = origin.startsWith('https://*.')
    const host = origin.slice(wildcard ? 'https://*.'.length : 'https://'.length).toLowerCase()
    return wildcard ? target.endsWith(`.${host}`) : target === host
  })
}

/** Header names a connector may never send: the browser sets them, or they carry who you are. */
const FORBIDDEN_SESSION_HEADERS = new Set([
  'cookie',
  'cookie2',
  'authorization',
  'host',
  'origin',
  'referer',
  'content-length'
])

const HEADER_NAME = /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/

/** Whether `name` is a header a connector may send inside its signed-in window, on a call or its check. */
export function allowedSessionHeader(name: string): boolean {
  const lower = name.toLowerCase()
  return (
    HEADER_NAME.test(name) &&
    !FORBIDDEN_SESSION_HEADERS.has(lower) &&
    !lower.startsWith('sec-') &&
    !lower.startsWith('proxy-')
  )
}
