import { existsSync } from 'node:fs'
import { mkdir, rm, stat } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { checkConnector, type CheckCode, type CheckFinding } from './check'
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
  WEB_DIR,
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
export type { BundleOutput, BundleRequest }

export interface PackOptions {
  /** Module specifier the connector was loaded from, bundled as the pack entry. */
  entry: string
  /** Directory the `.vorn.tgz` is written to; defaults to the working directory. */
  outDir?: string
  /** Directory module specifiers resolve from; defaults to the working directory. */
  resolveDir?: string
  /** SDK specifier the generated stdio entry imports; overridden in tests. */
  sdkModule?: string
  /** Size ceiling for the written archive; defaults to `MAX_PACK_BYTES`. */
  maxBytes?: number
  /** Size ceiling for what the archive unpacks to; defaults to `MAX_UNPACKED_BYTES`. */
  maxUnpackedBytes?: number
  /** Replaced in tests so packing does not shell out to a bundler. */
  bundle?(request: BundleRequest): Promise<BundleOutput>
  /** Replaced in tests whose subject is the archive rather than the launch; defaults to starting it for real. */
  launch?(dir: string): Promise<CheckFinding[]>
}

export interface PackResult {
  findings: CheckFinding[]
  /** Absolute path of the written pack; absent when a gate failed. */
  file?: string
  bytes?: number
}

/** A gate pack refuses on; the package-level ones say this for themselves. */
function finding(code: CheckCode, target: string, message: string): CheckFinding {
  return { level: 'error', code, target, message }
}

/** File name Vorn recognizes as a connector pack. */
export function packFileName(connector: Connector): string {
  return `${connector.id}-${connector.version}.vorn.tgz`
}

/** The entry is generated, not the author's bin, so every pack launches alike. */
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
  await mkdir(outDir, { recursive: true })
  const file = join(outDir, packFileName(connector))
  const staging = await stagePack(connector, built.code, packageRoot)
  try {
    // Asked of the staged files themselves: an artifact that cannot start is not one to ship.
    findings.push(...(await (options.launch ?? packLaunchFindings)(staging)))
    if (findings.some((item) => item.level === 'error')) return { findings }

    // Pages compress well, so a small archive can still unpack past what Vorn writes.
    const unpacked = await directoryBytes(staging)
    const maxUnpacked = options.maxUnpackedBytes ?? MAX_UNPACKED_BYTES
    if (unpacked > maxUnpacked) {
      return {
        findings: [
          ...findings,
          finding(
            'pack-too-large',
            'bundle',
            `The pack unpacks to ${Math.round(unpacked / 1024)} KB; Vorn unpacks at most ${Math.round(maxUnpacked / 1024)} KB`
          )
        ]
      }
    }

    const { create } = await import('tar')
    // What was staged, not what was declared; tarring a missing name fails with a path.
    await create({ gzip: true, file, cwd: staging }, [
      'manifest.json',
      'index.js',
      ...(existsSync(join(staging, WEB_DIR)) ? [WEB_DIR] : [])
    ])
  } finally {
    await rm(staging, { recursive: true, force: true })
  }

  const bytes = (await stat(file)).size
  const maxBytes = options.maxBytes ?? MAX_PACK_BYTES
  if (bytes > maxBytes) {
    await rm(file, { force: true })
    return {
      findings: [
        ...findings,
        finding(
          'pack-too-large',
          'bundle',
          `The pack is ${Math.round(bytes / 1024)} KB; Vorn installs at most ${Math.round(maxBytes / 1024)} KB`
        )
      ]
    }
  }
  return { findings, file, bytes }
}
