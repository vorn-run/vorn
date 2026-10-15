import { cp, mkdir, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { checkConnector, type CheckCode, type CheckFinding } from './check'
import type { ExtensionExtras } from './extension'
import {
  bundleDependencyFindings,
  bundledRequireFindings,
  esbuildBundle,
  lifecycleScriptFindings,
  packageDirFor,
  packageRootFor,
  packEntryContents,
  packLaunchFindings,
  readNearestPackageJson,
  stagePack,
  directoryBytes,
  MAX_PACK_BYTES,
  MAX_UNPACKED_BYTES,
  type BundleOutput,
  type BundleRequest
} from './packaging'
import type { Connector } from './types'

export {
  bundleDependencyFindings,
  bundledRequireFindings,
  lifecycleScriptFindings,
  readNearestPackageJson,
  MAX_PACK_BYTES,
  MAX_UNPACKED_BYTES
}
export { extrasOf }
export type { BundleOutput, BundleRequest }

export interface PackOptions {
  /** Module specifier the connector was loaded from, bundled as the pack entry. */
  entry: string
  /** Directory the package's folder is written into, as `<outDir>/<id>`; defaults to the working directory. */
  outDir?: string
  /** What the manifest declares beyond the code: hosts, the settings naming hosts, the publisher. Read from `vorn` in package.json when absent. */
  extras?: ExtensionExtras
  /** Directory module specifiers resolve from; defaults to the working directory. */
  resolveDir?: string
  /** SDK specifier the generated stdio entry imports; overridden in tests. */
  sdkModule?: string
  /** Size ceiling for what the archive unpacks to; defaults to `MAX_UNPACKED_BYTES`. */
  maxUnpackedBytes?: number
  /** Replaced in tests so packing does not shell out to a bundler. */
  bundle?(request: BundleRequest): Promise<BundleOutput>
  /** Replaced in tests whose subject is the archive rather than the launch; defaults to starting it for real. */
  launch?(dir: string): Promise<CheckFinding[]>
}

export interface PackResult {
  findings: CheckFinding[]
  /** Absolute path of the written folder, ready to be a git repository; absent when a gate failed. */
  dir?: string
  bytes?: number
}

/** A gate pack refuses on; the package-level ones say this for themselves. */
function finding(code: CheckCode, target: string, message: string): CheckFinding {
  return { level: 'error', code, target, message }
}

/** The manifest extras a package declares under `vorn` in its package.json. */
function extrasOf(pkg: Record<string, unknown> | undefined): ExtensionExtras {
  const vorn = (pkg?.vorn ?? {}) as Record<string, unknown>
  const strings = (v: unknown): string[] | undefined =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : undefined
  const author = pkg?.author
  const publisher =
    typeof vorn.publisher === 'string'
      ? vorn.publisher
      : typeof author === 'string'
        ? author.replace(/\s*<[^>]*>\s*$/, '')
        : undefined
  return { hosts: strings(vorn.hosts), hostsFrom: strings(vorn.hostsFrom), publisher }
}

/**
 * Builds the connector as a folder Vorn installs from git: its
 * `vorn-extension.toml`, one bundled `index.js` from a generated entry (so
 * every package launches alike), the receipt of its checks and the pages its
 * panes name. The folder is what its repository holds.
 */
export async function packConnector(
  connector: Connector,
  options: PackOptions
): Promise<PackResult> {
  const resolveDir = resolve(options.resolveDir ?? process.cwd())
  const entryDir = packageDirFor(resolveDir, options.entry)
  // Resolved once, so the gate and the archive never judge different packages.
  const packageRoot = packageRootFor(entryDir)

  const findings = await checkConnector(connector, { packageDir: packageRoot })
  findings.push(...lifecycleScriptFindings(readNearestPackageJson(entryDir)))
  if (findings.some((item) => item.level === 'error')) return { findings }

  const contents = packEntryContents(options.entry, options.sdkModule)
  const bundle = options.bundle ?? esbuildBundle
  const built = await bundle({ contents, resolveDir })
  findings.push(...bundleDependencyFindings(built.external), ...bundledRequireFindings(built.code))
  if (findings.some((item) => item.level === 'error')) return { findings }

  const outDir = resolve(options.outDir ?? process.cwd())
  const extras = options.extras ?? extrasOf(readNearestPackageJson(entryDir))
  const staging = await stagePack(connector, built.code, packageRoot, extras)
  try {
    // Asked of the staged files themselves: an artifact that cannot start is not one to ship.
    findings.push(...(await (options.launch ?? packLaunchFindings)(staging)))
    if (findings.some((item) => item.level === 'error')) return { findings }

    // What Vorn keeps of it on disk, which is what the host's ceiling measures.
    const bytes = await directoryBytes(staging)
    const maxUnpacked = options.maxUnpackedBytes ?? MAX_UNPACKED_BYTES
    if (bytes > maxUnpacked) {
      return {
        findings: [
          ...findings,
          finding(
            'pack-too-large',
            'bundle',
            `The package is ${Math.round(bytes / 1024)} KB; Vorn keeps at most ${Math.round(maxUnpacked / 1024)} KB`
          )
        ]
      }
    }
    const dir = join(outDir, connector.id)
    await mkdir(outDir, { recursive: true })
    // Only what the build makes is replaced; a repository's own files beside it stay.
    for (const made of ['index.js', 'vorn-extension.toml', 'verified.json', 'web']) {
      await rm(join(dir, made), { recursive: true, force: true })
    }
    await cp(staging, dir, { recursive: true })
    return { findings, dir, bytes }
  } finally {
    await rm(staging, { recursive: true, force: true })
  }
}
