/** Where a Vorn endpoint for this process is served; a URL keeps IPv6 in brackets. */
export const LOOPBACK_HOSTS = ['127.0.0.1', 'localhost', '[::1]']

export interface LoopbackNames {
  urlVar: string
  tokenVar: string
  /** Said when either variable is missing. */
  missing: string
  /** Names what is served, as in "the bridge is served on this machine". */
  served: string
}

/** Read an endpoint Vorn set in the environment, refusing one that is not on this machine. */
export function loopbackEndpoint(
  env: NodeJS.ProcessEnv,
  names: LoopbackNames,
  fail: (message: string) => Error = (message) => new Error(message)
): { url: string; token: string } {
  const url = env[names.urlVar]?.trim()
  const token = env[names.tokenVar]?.trim()
  if (!url || !token) throw fail(names.missing)
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw fail(`${names.urlVar} is ${JSON.stringify(url)}, which is not a URL`)
  }
  // Checked before the token is sent, so a variable pointing elsewhere cannot collect it.
  if (parsed.protocol !== 'http:' || !LOOPBACK_HOSTS.includes(parsed.hostname)) {
    throw fail(
      `${names.urlVar} is ${JSON.stringify(url)}; ${names.served}, over http on ${LOOPBACK_HOSTS.join(', ')}`
    )
  }
  return { url: url.replace(/\/$/, ''), token }
}
