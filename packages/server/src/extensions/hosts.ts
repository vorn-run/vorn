import { randomBytes } from 'node:crypto'
import type { Client } from '@modelcontextprotocol/sdk/client/index.js'
import type { InstalledConnectorPack } from '@vornrun/shared/types'
import { openMcpChild, type McpChild } from '../connectors/mcp-child'
import { installedLaunch, installedPack, listInstalledPacks } from '../connectors/packs'
import { createChildCache } from '../connectors/stdio-clients'
import { constantTimeEqual } from '../token-manager'

/**
 * One extension process per project, started when a session there first needs it.
 *
 * A connector's child is keyed by connection because that is what it polls. An
 * extension has no connection: it answers about the sessions of one project, so
 * the project is the key, and every session in it shares the one child.
 *
 * The token the child is given is minted here and never leaves this map. It is
 * what the bridge checks, so a second extension cannot answer as this one, and
 * a pane's page is never given it — a page proves itself with the nonce in its
 * own URL instead.
 */

export interface ExtensionHost {
  token: string
  extensionId: string
  projectPath: string
}

const hosts = createChildCache<McpChild, ExtensionHost>('extensions', (config, key) =>
  openMcpChild(config, key, 'extensions')
)

/** Where the bridge answers, learned once the server knows the port it won. */
let bridgeOrigin = ''

export function setExtensionBridgeOrigin(origin: string): void {
  bridgeOrigin = origin
}

export function extensionBridgeOrigin(): string {
  return bridgeOrigin
}

const keyOf = (extensionId: string, projectPath: string): string => `${extensionId} ${projectPath}`

/**
 * The extension a call claims to be from, matched by the token it was started with.
 *
 * Compared in constant time and without stopping at the first match: this is
 * the comparison that decides the caller, so it is the one that must not answer
 * a near-miss faster than a miss.
 */
export function hostByToken(extensionId: string, token: string): ExtensionHost | undefined {
  const offered = Buffer.from(token, 'utf8')
  let found: ExtensionHost | undefined
  for (const host of hosts.entries()) {
    if (host.extensionId !== extensionId) continue
    if (constantTimeEqual(Buffer.from(host.token, 'utf8'), offered)) found = host
  }
  return found
}

export function tokenFor(extensionId: string, projectPath: string): string | undefined {
  return hosts.find((host) => host.extensionId === extensionId && host.projectPath === projectPath)
    ?.token
}

export function isRunning(extensionId: string, projectPath: string): boolean {
  return hosts.has(keyOf(extensionId, projectPath))
}

/** Every installed extension, whatever any session makes of it. */
export function installedExtensions(): InstalledConnectorPack[] {
  try {
    return listInstalledPacks().filter((pack) => pack.kind === 'extension')
  } catch {
    // Before a data directory resolves there is nowhere to look, which reads as none installed.
    return []
  }
}

export async function getOrStartHost(extensionId: string, projectPath: string): Promise<Client> {
  const child = await hosts.getOrStart(keyOf(extensionId, projectPath), async () => {
    const pack = installedPack(extensionId)
    if (!pack || pack.kind !== 'extension') {
      throw new Error(`No extension "${extensionId}" is installed`)
    }
    const launch = installedLaunch(extensionId)
    if (!launch) throw new Error(`The extension "${extensionId}" has no files to run`)
    if (!bridgeOrigin) throw new Error('The extension bridge has no address yet')

    const token = randomBytes(32).toString('base64url')
    return {
      config: {
        command: launch.command,
        args: launch.args,
        cwd: projectPath,
        // The two names the SDK's bridge client reads, and nothing else about this machine.
        env: {
          VORN_EXTENSION_HOST: `${bridgeOrigin}/extensions/${extensionId}/bridge`,
          VORN_EXTENSION_TOKEN: token
        }
      },
      meta: { token, extensionId, projectPath }
    }
  })
  return child.client
}

export async function stopHost(extensionId: string, projectPath: string): Promise<void> {
  await hosts.stop(keyOf(extensionId, projectPath))
}

/** A child started before a pack changed keeps running the old files until stopped. */
export async function stopHostsForExtension(extensionId: string): Promise<void> {
  await hosts.stopWhere((host) => host.extensionId === extensionId)
}

export async function stopHostsForProject(projectPath: string): Promise<void> {
  await hosts.stopWhere((host) => host.projectPath === projectPath)
}

export async function stopAllHosts(): Promise<void> {
  await hosts.stopAll()
}
