/**
 * Token acquisition for Azure Data Explorer.
 *
 * Kusto has no equivalent of a personal access token — every request is
 * authenticated with Microsoft Entra ID. `DefaultAzureCredential` walks the
 * usual chain (environment service principal, workload identity, managed
 * identity, Azure CLI, Azure Developer CLI, …), so a developer who has run
 * `az login` and a service running with a managed identity both work without
 * the connector knowing which one it got.
 */

/** Minimal shape of the credential, so tests can supply their own. */
export interface TokenCredentialLike {
  getToken(scope: string): Promise<{ token: string; expiresOnTimestamp?: number } | null>
}

export type CredentialFactory = () => TokenCredentialLike

/**
 * Refresh this long before a token actually expires. Entra tokens are
 * typically valid for an hour, and a poll that starts just under the wire
 * would otherwise fail against a clock skew of a few seconds.
 */
const EXPIRY_MARGIN_MS = 5 * 60 * 1000

let cachedFactory: CredentialFactory | undefined

/**
 * Loaded lazily so that importing the connector — which `vorn-connector check`
 * and the unit tests both do — never pulls in the Azure identity chain, and so
 * a missing optional dependency surfaces on first use rather than at import.
 */
async function defaultCredentialFactory(): Promise<CredentialFactory> {
  if (!cachedFactory) {
    const { DefaultAzureCredential } = await import('@azure/identity')
    const credential = new DefaultAzureCredential()
    cachedFactory = () => credential
  }
  return cachedFactory
}

interface CachedToken {
  token: string
  expiresAt: number
}

/**
 * Acquire bearer tokens for one cluster, reusing them until they near expiry.
 *
 * The connector runs as a long-lived stdio process polling on a timer, so
 * without caching every poll would pay a credential-chain probe — which for
 * the Azure CLI credential means spawning a process.
 */
export function createTokenProvider(
  options: {
    credential?: TokenCredentialLike
    now?: () => number
  } = {}
): (clusterUrl: string) => Promise<string> {
  const now = options.now ?? (() => Date.now())
  const cache = new Map<string, CachedToken>()

  return async function getToken(clusterUrl: string): Promise<string> {
    const scope = `${clusterUrl}/.default`
    const cached = cache.get(scope)
    if (cached && cached.expiresAt - EXPIRY_MARGIN_MS > now()) return cached.token

    const credential = options.credential ?? (await defaultCredentialFactory())()
    let result: { token: string; expiresOnTimestamp?: number } | null
    try {
      result = await credential.getToken(scope)
    } catch (error) {
      throw new Error(
        `Could not get an Azure token for ${clusterUrl}. Sign in with \`az login\`, or set ` +
          `AZURE_CLIENT_ID / AZURE_TENANT_ID / AZURE_CLIENT_SECRET for a service principal. ` +
          `(${error instanceof Error ? error.message : String(error)})`,
        { cause: error }
      )
    }
    if (!result?.token) {
      throw new Error(
        `No Azure credential was available for ${clusterUrl}. Sign in with \`az login\`, or set ` +
          `AZURE_CLIENT_ID / AZURE_TENANT_ID / AZURE_CLIENT_SECRET for a service principal.`
      )
    }

    cache.set(scope, {
      token: result.token,
      // Treat an unknown expiry as immediately stale rather than caching it
      // forever; a re-probe is cheap next to silently using a dead token.
      expiresAt: result.expiresOnTimestamp ?? 0
    })
    return result.token
  }
}
