/**
 * Connector packages Vorn knows about by name.
 *
 * A packaged connector is only discoverable if you already know its package
 * name, which makes a first-party connector harder to add than a third-party
 * MCP server. The catalog is what lets one be presented like any other
 * connector: a name, an icon and an Add button, with the package name an
 * implementation detail rather than something to memorize.
 *
 * Entries carry their own icon and blurb so the list renders without starting
 * anything. Everything a *connection* needs — triggers, config, poll filters —
 * still comes from probing the package itself, so this stays a pointer and
 * cannot drift into a second, stale copy of a connector's definition.
 */
import { existsSync } from 'fs'
import { join } from 'path'
import type { ConnectorCatalogEntry, ConnectorCatalogItem } from '@vornrun/shared/types'

export const CONNECTOR_CATALOG: ConnectorCatalogEntry[] = [
  {
    id: 'kusto',
    name: 'Azure Data Explorer',
    description: 'Trigger workflows from the rows a KQL query returns.',
    packageName: '@vornrun/connector-kusto',
    capabilities: ['triggers', 'actions'],
    category: 'Data & observability',
    keywords: ['kusto', 'adx', 'azure', 'kql', 'query', 'logs', 'telemetry'],
    auth: 'Signs in with your Azure identity — `az login` is usually all it needs.',
    icon: {
      viewBox: '0 0 24 24',
      paths: [
        'M12 2C7.6 2 4 3.6 4 5.5S7.6 9 12 9s8-1.6 8-3.5S16.4 2 12 2zm0 5.5c-3.6 0-6-1.2-6-2s2.4-2 6-2 6 1.2 6 2-2.4 2-6 2z',
        'M4 9.4v2.1c0 1.9 3.6 3.5 8 3.5.5 0 1 0 1.4-.1a5.6 5.6 0 01.8-1.9c-.7.1-1.5.2-2.2.2-3.6 0-6-1.2-6-2V9.4z',
        'M4 15.4v2.1C4 19.4 7.6 21 12 21c.3 0 .6 0 .9-.1a5.6 5.6 0 01-1-1.9H12c-3.6 0-6-1.2-6-2v-1.6z',
        'M17.5 13a4.5 4.5 0 103.1 7.7l1.6 1.6a1 1 0 001.4-1.4l-1.6-1.6A4.5 4.5 0 0017.5 13zm0 2a2.5 2.5 0 110 5 2.5 2.5 0 010-5z'
      ]
    }
  }
]

/**
 * Where a catalog entry is launched from.
 *
 * Normally `npx -y`, which resolves the package at run time so a connector is
 * never bundled into the app and an upgrade is a version bump rather than a
 * release.
 *
 * A local build wins only when the launcher says a checkout is being run,
 * because otherwise working on a connector would mean testing whatever is
 * published rather than the code just changed — and a connector that has not
 * shipped yet could not be run at all. Sniffing the working directory instead
 * would make a released app pick up a stray `packages/` folder it happened to
 * be started next to, so the repo root is passed in rather than guessed.
 */
export function catalogLaunchSpec(
  entry: ConnectorCatalogEntry,
  repoRoot: string | undefined = process.env.VORN_REPO_ROOT
): { command: string; args: string[] } {
  if (repoRoot) {
    const local = join(repoRoot, 'packages', localPackageDir(entry.packageName), 'dist', 'index.js')
    if (existsSync(local)) return { command: 'node', args: [local] }
  }
  return { command: 'npx', args: ['-y', entry.packageName] }
}

/** `@vornrun/connector-kusto` lives in `packages/connector-kusto`. */
function localPackageDir(packageName: string): string {
  return packageName.replace(/^@[^/]+\//, '')
}

/**
 * The catalog as the UI consumes it, resolved once.
 *
 * Neither the catalog nor the repo root changes while the process runs, so
 * resolving per request would re-stat the filesystem for every entry on every
 * settings page load.
 */
export function catalogItems(): ConnectorCatalogItem[] {
  resolved ??= CONNECTOR_CATALOG.map((entry) => ({ ...entry, launch: catalogLaunchSpec(entry) }))
  return resolved
}

let resolved: ConnectorCatalogItem[] | undefined
