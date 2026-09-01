import type { ActionResult, ConnectorManifest, VornConnector } from '@vornrun/shared/types'

/** Auth-profile fields; `secret` arrives already decrypted from the creds store. */
export interface HttpProfileFields {
  baseUrl?: unknown
  authHeader?: unknown
  authQuery?: unknown
  authBody?: unknown
  secret?: unknown
}

export interface HttpRequestSpec {
  method: string
  url: string
  headers?: Record<string, string>
  body?: string
}

function injectSecret(template: string, secret: string): string {
  return template.replaceAll('{{secret}}', secret)
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

const ALLOWED_METHODS = new Set(['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'])

/**
 * A profile whose password fields never made it through decryption still holds
 * ciphertext; sending that as a bearer token is a silent, confusing failure.
 */
export function lockedProfileError(
  filters: Record<string, unknown>,
  decrypted: Record<string, string> | undefined
): string | null {
  if (typeof filters.secret !== 'string' || !filters.secret) return null
  if (decrypted?.secret !== undefined) return null
  return "This profile's secret is locked - decryption is unavailable or has not synced yet."
}

export const HTTP_CONNECTOR_ID = 'http'

/** Why this connection cannot sign a request: it is not an auth profile, or its secret is locked. */
export function httpProfileError(
  conn: { connectorId: string; filters: Record<string, unknown> },
  decrypted: Record<string, string> | undefined
): string | null {
  if (conn.connectorId !== HTTP_CONNECTOR_ID)
    return `Connection belongs to the ${conn.connectorId} connector, not an HTTP auth profile`
  return lockedProfileError(conn.filters, decrypted)
}

/**
 * Execute one HTTP request with a profile's injection applied. This runs in
 * the server so the secret never crosses into the renderer and responses skip
 * browser CORS entirely.
 */
export async function performHttpRequest(
  profile: HttpProfileFields,
  spec: HttpRequestSpec
): Promise<ActionResult> {
  const method = asString(spec.method).toUpperCase()
  if (!ALLOWED_METHODS.has(method)) {
    return { success: false, error: `Invalid HTTP method: ${spec.method || '(none)'}` }
  }
  const secret = asString(profile.secret)
  let url: URL
  try {
    url = new URL(spec.url, asString(profile.baseUrl) || undefined)
  } catch {
    return { success: false, error: `Invalid URL: ${spec.url}` }
  }

  // The URL is template-resolved, so an absolute URL can point anywhere; a
  // profile's auth only ever travels to the origin the profile names.
  const hasInjection = !!(
    asString(profile.authHeader) ||
    asString(profile.authQuery) ||
    asString(profile.authBody).trim() ||
    secret
  )
  if (hasInjection) {
    let baseOrigin: string | null
    try {
      const base = asString(profile.baseUrl)
      baseOrigin = base ? new URL(base).origin : null
    } catch {
      baseOrigin = null
    }
    if (!baseOrigin || url.origin !== baseOrigin) {
      return {
        success: false,
        error: `This profile only signs requests to ${baseOrigin ?? 'its base URL (none set)'}; refusing ${url.origin}`
      }
    }
  }

  const headers: Record<string, string> = { ...(spec.headers ?? {}) }
  const authHeader = asString(profile.authHeader)
  if (authHeader.includes(':')) {
    const i = authHeader.indexOf(':')
    headers[authHeader.slice(0, i).trim()] = injectSecret(authHeader.slice(i + 1).trim(), secret)
  }
  const authQuery = asString(profile.authQuery)
  if (authQuery.includes('=')) {
    const i = authQuery.indexOf('=')
    url.searchParams.set(
      authQuery.slice(0, i).trim(),
      injectSecret(authQuery.slice(i + 1).trim(), secret)
    )
  }

  let body = spec.body
  const authBody = asString(profile.authBody).trim()
  if (authBody) {
    try {
      const inject: unknown = JSON.parse(injectSecret(authBody, secret))
      const current: unknown = body?.trim() ? JSON.parse(body) : {}
      const bothObjects =
        inject !== null &&
        typeof inject === 'object' &&
        !Array.isArray(inject) &&
        current !== null &&
        typeof current === 'object' &&
        !Array.isArray(current)
      if (bothObjects) {
        body = JSON.stringify({ ...(current as object), ...(inject as object) })
        const hasContentType = Object.keys(headers).some((k) => k.toLowerCase() === 'content-type')
        if (!hasContentType) headers['Content-Type'] = 'application/json'
      }
    } catch {
      // A body that is not a JSON object passes through untouched.
    }
  }

  const sendBody = body && method !== 'GET' && method !== 'HEAD'
  try {
    const res = await fetch(url, {
      method,
      headers,
      ...(sendBody ? { body } : {}),
      // A redirect could carry the auth to a different host; report it instead.
      redirect: 'manual',
      signal: AbortSignal.timeout(30_000)
    })
    const text = await res.text()
    let parsed: unknown = text
    try {
      parsed = JSON.parse(text)
    } catch {
      // Non-JSON responses stay as text.
    }
    const responseHeaders: Record<string, string> = {}
    res.headers.forEach((value, key) => {
      responseHeaders[key] = value
    })
    return { success: true, output: { status: res.status, headers: responseHeaders, body: parsed } }
  } catch (err) {
    return { success: false, error: err instanceof Error ? err.message : String(err) }
  }
}

/**
 * Auth profiles are connections of this connector: a base URL plus injection
 * rules that carry a secret into every request made through the profile.
 */
export const httpConnector: VornConnector = {
  id: 'http',
  name: 'HTTP',
  icon: 'http',
  capabilities: ['actions'],

  async execute(actionType: string, args: Record<string, unknown>): Promise<ActionResult> {
    const profile = args as HttpProfileFields
    if (actionType === 'test') {
      const baseUrl = asString(profile.baseUrl)
      if (!baseUrl) return { success: false, error: 'Set a base URL to test this profile' }
      return performHttpRequest(profile, { method: 'GET', url: baseUrl })
    }
    if (actionType === 'request') {
      const url = asString(args.url)
      if (!url) return { success: false, error: 'url is required' }
      const headers =
        args.headers && typeof args.headers === 'object' && !Array.isArray(args.headers)
          ? (args.headers as Record<string, string>)
          : undefined
      return performHttpRequest(profile, {
        method: asString(args.method) || 'GET',
        url,
        headers,
        body: asString(args.body) || undefined
      })
    }
    return { success: false, error: `Unknown action: ${actionType}` }
  },

  describe(): ConnectorManifest {
    return {
      auth: [
        {
          key: 'profileName',
          label: 'Profile name',
          type: 'text',
          required: true,
          placeholder: 'Acme API'
        },
        { key: 'baseUrl', label: 'Base URL', type: 'text', placeholder: 'https://api.example.com' },
        {
          key: 'authHeader',
          label: 'Header added to every request',
          type: 'text',
          placeholder: 'Authorization: Bearer {{secret}}'
        },
        {
          key: 'authQuery',
          label: 'Query parameter added to every request',
          type: 'text',
          placeholder: 'api_key={{secret}}'
        },
        {
          key: 'authBody',
          label: 'JSON merged into every request body',
          type: 'text',
          placeholder: '{"token": "{{secret}}"}'
        },
        {
          key: 'secret',
          label: 'Secret',
          type: 'password',
          description: 'Stored encrypted. {{secret}} in the fields above stands for this value.'
        }
      ],
      actions: [
        {
          type: 'request',
          label: 'HTTP request',
          description: 'Send a request through this profile',
          configFields: [
            {
              key: 'method',
              label: 'Method',
              type: 'select',
              options: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'].map((m) => ({
                value: m,
                label: m
              }))
            },
            {
              key: 'url',
              label: 'URL or path',
              type: 'text',
              required: true,
              placeholder: '/v1/items',
              supportsTemplates: true
            },
            { key: 'body', label: 'Body', type: 'textarea', supportsTemplates: true }
          ],
          outputSchema: {
            type: 'object',
            properties: {
              status: { type: 'number' },
              headers: { type: 'object' },
              body: { type: ['object', 'array', 'string', 'number', 'boolean', 'null'] }
            }
          }
        }
      ]
    }
  }
}
