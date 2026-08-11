/**
 * The connectors Vorn can offer, fetched from the connectors repository.
 *
 * A packaged connector is only discoverable if you already know its package
 * name, which makes a first-party connector harder to add than a third-party
 * MCP server. The catalog is what lets one be presented like any other: a name,
 * an icon and an Add button, with the package name an implementation detail.
 *
 * The list is fetched rather than compiled in, because connectors ship on their
 * own schedule now — one published this afternoon should be offered without
 * waiting for an app release. A copy is bundled so a first run with no network
 * still shows something, and the last good fetch is cached on disk so a later
 * one does not depend on being online either.
 *
 * Entries carry enough to decide with — what a connector fires on, what a
 * workflow can ask it to do, what it will want configured — because all of that
 * is generated from the connector's own manifest upstream. What a *connection*
 * needs is still read by probing the installed package, so nothing here is
 * trusted once a connector is actually being set up.
 */
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'fs'
import { join, dirname } from 'path'
import os from 'os'
import type { ConnectorCatalogEntry, ConnectorCatalogItem } from '@vornrun/shared/types'

/**
 * Where the published catalog lives.
 *
 * The raw file on the default branch, so publishing a connector and updating
 * the catalog is one commit rather than a release step somebody has to remember.
 */
const CATALOG_URL = 'https://raw.githubusercontent.com/vorn-run/connectors/main/catalog.json'

/** Long enough that opening settings twice does not fetch twice. */
const MAX_AGE_MS = 6 * 60 * 60 * 1000

/** A slow network must not hold up the connector list. */
const FETCH_TIMEOUT_MS = 5000

const CACHE_PATH = join(os.homedir(), '.vorn', 'connector-catalog.json')

/**
 * Enough to show a connector list on a first run with no network.
 *
 * A seed, not a copy: it carries what the list renders and nothing more, and it
 * is replaced by the published catalog as soon as one is fetched. It does not
 * need to be current, and should not be extended to track the real one — that
 * is the drift this file exists to avoid.
 */
export const CONNECTOR_CATALOG: ConnectorCatalogEntry[] = [
  {
    id: 'ado',
    name: 'Azure DevOps',
    description: 'Trigger workflows from the work items a WIQL query returns.',
    packageName: '@vornrun/connector-ado',
    capabilities: ['triggers'],
    category: 'Development',
    keywords: ['azure devops', 'ado', 'work items', 'wiql', 'boards', 'tfs'],
    auth: 'Signs in with your Azure identity — `az login` is usually all it needs.',
    icon: {
      viewBox: '0 0 24 24',
      paths: [
        'M4 4h5v16H4a1 1 0 01-1-1V5a1 1 0 011-1z',
        'M10.5 4h3v9h-3z',
        'M15 4h5a1 1 0 011 1v14a1 1 0 01-1 1h-5z'
      ]
    }
  },
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

interface CachedCatalog {
  fetchedAt: number
  connectors: ConnectorCatalogEntry[]
}

/**
 * Read a catalog document, or refuse it.
 *
 * This comes off the network, so a truncated response or a future format has to
 * fail into "use what we had" rather than into an empty connector list. Entries
 * missing what the list needs to render are dropped individually, so one bad
 * connector upstream does not hide the rest.
 */
export function parseCatalog(document: unknown): ConnectorCatalogEntry[] | undefined {
  const root = document as { version?: unknown; connectors?: unknown }
  if (root?.version !== 1 || !Array.isArray(root.connectors)) return undefined

  const entries = root.connectors.filter(
    (entry): entry is ConnectorCatalogEntry =>
      typeof entry?.id === 'string' &&
      typeof entry?.name === 'string' &&
      typeof entry?.packageName === 'string' &&
      // Anything else is decoration; without a package there is nothing to add.
      entry.packageName.length > 0
  )
  return entries.length > 0 ? entries : undefined
}

/**
 * Where a catalog entry is launched from.
 *
 * Normally `npx -y`, which resolves the package at run time so a connector is
 * never bundled into the app and an upgrade is a version bump rather than a
 * release.
 *
 * A local build wins when `VORN_CONNECTORS_ROOT` names a checkout of the
 * connectors repository, because otherwise working on a connector would mean
 * testing whatever is published rather than the code just changed — and one
 * that has not shipped yet could not be run at all. It is set deliberately
 * rather than sniffed from the working directory, which would make a released
 * app pick up a stray `packages/` folder it happened to be started next to.
 */
export function catalogLaunchSpec(
  entry: ConnectorCatalogEntry,
  repoRoot: string | undefined = process.env.VORN_CONNECTORS_ROOT
): { command: string; args: string[] } {
  if (repoRoot) {
    const local = join(repoRoot, 'packages', localPackageDir(entry.packageName), 'dist', 'index.js')
    if (existsSync(local)) return { command: 'node', args: [local] }
  }
  return { command: 'npx', args: ['-y', entry.packageName] }
}

/** `@vornrun/connector-kusto` lives in `packages/kusto`. */
function localPackageDir(packageName: string): string {
  return packageName.replace(/^@[^/]+\/(connector-)?/, '')
}

/**
 * Where the impure parts come from.
 *
 * Defaulted for production and supplied by tests, so a unit test never reaches
 * the network or reads whatever this machine happens to have cached.
 */
export interface CatalogOptions {
  now?: number
  cachePath?: string
  fetchImpl?: typeof fetch
}

function readCache(cachePath: string): CachedCatalog | undefined {
  try {
    const cached = JSON.parse(readFileSync(cachePath, 'utf8')) as CachedCatalog
    const connectors = parseCatalog({ version: 1, connectors: cached?.connectors })
    return connectors ? { fetchedAt: Number(cached.fetchedAt) || 0, connectors } : undefined
  } catch {
    // No cache yet, or one written by a version that shaped it differently.
    return undefined
  }
}

function writeCache(cachePath: string, connectors: ConnectorCatalogEntry[], now: number): void {
  try {
    mkdirSync(dirname(cachePath), { recursive: true })
    writeFileSync(cachePath, JSON.stringify({ fetchedAt: now, connectors }, null, 2))
  } catch {
    // A read-only home directory costs a fetch per session, nothing more.
  }
}

async function download(get: typeof fetch): Promise<ConnectorCatalogEntry[] | undefined> {
  try {
    const response = await get(CATALOG_URL, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) })
    if (!response.ok) return undefined
    return parseCatalog(await response.json())
  } catch {
    // Offline, blocked by a proxy, or serving something that is not the
    // catalog. All of them mean the same thing here: keep what we have.
    return undefined
  }
}

let resolved: ConnectorCatalogItem[] | undefined

/**
 * The catalog as the UI consumes it.
 *
 * Serves whatever is already known immediately — cache, else the bundled copy —
 * and refreshes in the background when that is stale, so opening the connector
 * list never waits on the network. The newer list appears the next time it is
 * opened, which for a list that changes a few times a week is soon enough.
 */
export function catalogItems(options: CatalogOptions = {}): ConnectorCatalogItem[] {
  if (!resolved) {
    const now = options.now ?? Date.now()
    const cache = readCache(options.cachePath ?? CACHE_PATH)
    resolved = withLaunch(cache?.connectors ?? CONNECTOR_CATALOG)
    if (!cache || now - cache.fetchedAt > MAX_AGE_MS) void refreshCatalog(options)
  }
  return resolved
}

/** Fetch the published catalog and adopt it if it parses. Never throws. */
export async function refreshCatalog(options: CatalogOptions = {}): Promise<boolean> {
  const connectors = await download(options.fetchImpl ?? fetch)
  if (!connectors) return false
  writeCache(options.cachePath ?? CACHE_PATH, connectors, options.now ?? Date.now())
  resolved = withLaunch(connectors)
  return true
}

function withLaunch(entries: ConnectorCatalogEntry[]): ConnectorCatalogItem[] {
  return entries.map((entry) => ({ ...entry, launch: catalogLaunchSpec(entry) }))
}

/** Test seam: forget what has been resolved this process. */
export function resetCatalogCache(): void {
  resolved = undefined
}
