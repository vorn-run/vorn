import type { SdkConnectorAuth, SdkConnectorManifest } from '@vornrun/shared/types'
import { describePack } from './packs'
import { localLaunchSpec } from './catalog'
import { probeSdkConnector } from './sdk-probe'
import log from '../logger'

/**
 * How a packaged connector signs in, whichever copy of it is going to run.
 *
 * `resolveLaunch` prefers a local checkout over an installed pack, and the auth
 * has to follow the same order or a connector run from a checkout — which is
 * how every connector is developed — would borrow nothing and report that its
 * rung has nothing to check. A checkout has no installed manifest to read, so
 * it is asked for one, once.
 */
export interface ConnectorBorrow {
  auth: SdkConnectorAuth
  /** The variables the connector declares reading, which bound what it may borrow. */
  declared: string[]
}

/** Probed manifests by connector id; `null` records a connector that would not answer. */
const checkoutManifests = new Map<string, SdkConnectorManifest | null>()

export function resetCheckoutAuthCache(): void {
  checkoutManifests.clear()
}

function borrowFrom(manifest: {
  auth?: SdkConnectorAuth
  env: Array<{ name: string }>
}): ConnectorBorrow | undefined {
  if (manifest.auth?.rung !== 'cli') return undefined
  return { auth: manifest.auth, declared: manifest.env.map((entry) => entry.name) }
}

/** What is installed for a connector, or nothing when there is nowhere to look. */
function installed(sdkId: string): ReturnType<typeof describePack> {
  try {
    return describePack(sdkId)
  } catch {
    // No data directory yet, so nothing is installed to read.
    return undefined
  }
}

/**
 * Whether resolving this connector's auth is worth suspending for.
 *
 * Answered synchronously so the common case — a connector that borrows nothing
 * — reaches its spawn in the tick that asked for it.
 */
export function mightBorrow(sdkId: string): boolean {
  if (!sdkId) return false
  const pack = installed(sdkId)
  if (pack) return pack.auth?.rung === 'cli'
  // A checkout cannot be read without asking, and a cached answer is still an
  // answer: only an unprobed or borrowing checkout is worth the wait.
  const cached = checkoutManifests.get(sdkId)
  if (cached !== undefined) return cached !== null && borrowFrom(cached) !== undefined
  return localLaunchSpec(sdkId) !== undefined
}

export async function resolveBorrow(sdkId: string): Promise<ConnectorBorrow | undefined> {
  if (!sdkId) return undefined
  const pack = installed(sdkId)
  if (pack) return borrowFrom(pack)

  const cached = checkoutManifests.get(sdkId)
  if (cached !== undefined) return cached ? borrowFrom(cached) : undefined

  const launch = localLaunchSpec(sdkId)
  if (!launch) return undefined

  const result = await probeSdkConnector(launch)
  if (!result.ok) {
    // Remembered as a no rather than retried on every spawn: a checkout that
    // will not start is a development problem, not a reason to keep paying for
    // a child process before every launch.
    checkoutManifests.set(sdkId, null)
    log.warn(`[auth] could not read ${sdkId} from its checkout: ${result.error}`)
    return undefined
  }
  checkoutManifests.set(sdkId, result.manifest)
  return borrowFrom(result.manifest)
}
