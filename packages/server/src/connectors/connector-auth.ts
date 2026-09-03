import type { SdkConnectorAuth } from '@vornrun/shared/types'
import { installedPack } from './packs'
import { localLaunchSpec } from './catalog'
import { probeSdkConnector } from './sdk-probe'
import log from '../logger'

// How a connector signs in, read from whichever copy of it resolveLaunch will run: checkout first.
export interface ConnectorAuthSource {
  auth: SdkConnectorAuth | undefined
  /** The variables the connector declares reading, which bound what it may borrow. */
  declared: string[]
  /** Only an auth block the app itself wrote may name a credential. */
  trusted: boolean
}

type Described = { auth?: SdkConnectorAuth; env: Array<{ name: string }> }

const checkoutAuth = new Map<string, ConnectorAuthSource | null>()

function sourceFrom(manifest: Described): ConnectorAuthSource {
  return { auth: manifest.auth, declared: manifest.env.map((entry) => entry.name), trusted: false }
}

export async function resolveConnectorAuth(
  sdkId: string
): Promise<ConnectorAuthSource | undefined> {
  if (!sdkId) return undefined
  const launch = localLaunchSpec(sdkId)
  if (!launch) {
    const pack = installedPack(sdkId)
    return pack ? sourceFrom(pack) : undefined
  }

  const cached = checkoutAuth.get(sdkId)
  if (cached !== undefined) return cached ?? undefined

  const result = await probeSdkConnector(launch)
  if (!result.ok) {
    // Remembered as a no: a checkout that will not start is not worth a child before every spawn.
    checkoutAuth.set(sdkId, null)
    log.warn(`[auth] could not read ${sdkId} from its checkout: ${result.error}`)
    return undefined
  }
  const source = sourceFrom(result.manifest)
  checkoutAuth.set(sdkId, source)
  return source
}
