// Mirrors the connector SDK's origin rule, which the published SDK cannot import from here.

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
