import type { ExtensionHost, ExtensionHostMethod, ExtensionUsage } from './types'

/**
 * How an extension asks the host for what it was granted.
 *
 * The extension runs as its own process, so the bridge is an HTTP endpoint the
 * host serves and names in the environment, with a token that says which
 * extension is calling. The host grants exactly the permissions the manifest
 * declared, which is why a call outside them comes back refused rather than
 * empty — the same answer the check's stub gives, so an extension meets the
 * rule once rather than twice.
 */

/** Where the host answers, and the token that says who is asking. */
export const HOST_URL_ENV = 'VORN_EXTENSION_HOST'
export const HOST_TOKEN_ENV = 'VORN_EXTENSION_TOKEN'

/** The host refused a call the extension's manifest never asked for. */
export class PermissionDeniedError extends Error {
  constructor(method: string, detail: string) {
    super(`The host refused ${method}: ${detail}`)
    this.name = 'PermissionDeniedError'
  }
}

/** The host answered, but with something this bridge cannot read as a result. */
export class HostReplyError extends Error {
  constructor(method: string, detail: string) {
    super(`The host answered ${method} with ${detail}`)
    this.name = 'HostReplyError'
  }
}

export interface HostBridgeOptions {
  sessionId: string
  env?: NodeJS.ProcessEnv
  /** Replaced in tests so nothing opens a socket. */
  fetchImpl?: typeof fetch
}

/** Long enough for a git read on a large tree, short enough to fail a wedged host. */
const HOST_TIMEOUT_MS = 15_000

/** The bridge is served on this machine, so a token never leaves it. */
const LOOPBACK_HOSTS = ['127.0.0.1', 'localhost', '::1']

function endpoint(env: NodeJS.ProcessEnv): { url: string; token: string } {
  const url = env[HOST_URL_ENV]?.trim()
  const token = env[HOST_TOKEN_ENV]?.trim()
  if (!url || !token) {
    throw new Error(
      `This extension was started without a host bridge; ${HOST_URL_ENV} and ${HOST_TOKEN_ENV} are set by Vorn`
    )
  }
  // Checked before the token is sent: a bridge address that is not this machine
  // would hand the grant to whoever set the variable.
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw new Error(`${HOST_URL_ENV} is ${JSON.stringify(url)}, which is not a URL`)
  }
  if (parsed.protocol !== 'http:' || !LOOPBACK_HOSTS.includes(parsed.hostname)) {
    throw new Error(
      `${HOST_URL_ENV} is ${JSON.stringify(url)}; the bridge is served on this machine, over http on ${LOOPBACK_HOSTS.join(', ')}`
    )
  }
  return { url: url.replace(/\/$/, ''), token }
}

/** The host as an extension process reaches it, over the bridge Vorn served it. */
export function createExtensionHost(options: HostBridgeOptions): ExtensionHost {
  const env = options.env ?? process.env
  const call = options.fetchImpl ?? fetch

  async function ask<T>(method: ExtensionHostMethod, params: Record<string, unknown>): Promise<T> {
    const { url, token } = endpoint(env)
    const response = await call(`${url}/${method}`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ sessionId: options.sessionId, ...params }),
      signal: AbortSignal.timeout(HOST_TIMEOUT_MS)
    })
    const text = await response.text()
    if (response.status === 403) throw new PermissionDeniedError(method, text || 'not granted')
    if (!response.ok) throw new Error(`The host answered ${method} with HTTP ${response.status}`)
    if (text === '') return undefined as T
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch {
      throw new HostReplyError(method, 'a body that is not JSON')
    }
    if (!parsed || typeof parsed !== 'object' || !('result' in parsed)) {
      throw new HostReplyError(method, 'a body carrying no result')
    }
    return (parsed as { result: T }).result
  }

  return {
    diff: () => ask('diff', {}),
    status: () => ask('status', {}),
    output: (opts) => ask('output', { ...(opts?.lines !== undefined && { lines: opts.lines }) }),
    selection: () => ask('selection', {}),
    send: (text) => ask('send', { text }),
    rename: (name) => ask('rename', { name }),
    usage: () => ask<ExtensionUsage>('usage', {})
  }
}
