import os from 'node:os'
import { getCurrentHost } from './server-rebind'

/**
 * Where the web client can actually be reached right now.
 *
 * This exists because Tailscale used to be the only answer: `appUrl` was built
 * from the tailnet address and only when Tailscale was running, so enabling
 * remote access without it left the user told the feature was on but not where to
 * point a browser.
 *
 * Addresses are enumerated rather than remembered. A machine's addresses change
 * on every DHCP renewal, wifi switch and VPN toggle, and there is no event for
 * any of it — so anything cached here would be wrong exactly when someone moved
 * networks and needed it most.
 */
export interface ReachableUrls {
  /** Empty while bound to loopback: nothing else can reach it. */
  urls: string[]
  port: number
  /** Whether the server is bound wide. The UI says different things either way. */
  remote: boolean
}

/**
 * IPv4 only, and no link-local.
 *
 * IPv6 link-local addresses need a zone id (`%en0`) to be usable, which is not
 * expressible in a URL — offering one would produce a link that cannot be opened.
 * The same goes for the `169.254.x` block, which means DHCP failed.
 */
function lanAddresses(): string[] {
  const found: string[] = []
  for (const addrs of Object.values(os.networkInterfaces())) {
    for (const addr of addrs ?? []) {
      if (addr.family !== 'IPv4' || addr.internal) continue
      if (addr.address.startsWith('169.254.')) continue
      found.push(addr.address)
    }
  }
  return found
}

/**
 * Tailscale addresses sort first when present: it is still the recommended way to
 * reach the server, because it is the only one of these that is encrypted.
 */
export function reachableUrls(port: number, tailscaleIps: string[] = []): ReachableUrls {
  const remote = getCurrentHost() === '0.0.0.0'
  if (!remote) return { urls: [], port, remote }

  const hosts = [...tailscaleIps.filter(Boolean), ...lanAddresses()]
  const seen = new Set<string>()
  const urls: string[] = []
  for (const host of hosts) {
    if (seen.has(host)) continue
    seen.add(host)
    urls.push(`http://${host}:${port}/app/`)
  }
  return { urls, port, remote }
}
