import { mkdtemp, mkdir, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { checkConnector, type CheckCode, type CheckFinding } from './check'
import {
  bundleDependencyFindings,
  esbuildBundle,
  lifecycleScriptFindings,
  packEntryContents,
  readNearestPackageJson,
  MAX_PACK_BYTES,
  type BundleOutput,
  type BundleRequest
} from './packaging'
import { connectorManifest } from './setup'
import type { Connector } from './types'

export { bundleDependencyFindings, lifecycleScriptFindings, readNearestPackageJson, MAX_PACK_BYTES }
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
  /** Replaced in tests so packing does not shell out to a bundler. */
  bundle?(request: BundleRequest): Promise<BundleOutput>
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
  const entryDir =
    options.entry.startsWith('.') || isAbsolute(options.entry)
      ? dirname(resolve(resolveDir, options.entry))
      : resolveDir

  const findings = await checkConnector(connector)
  findings.push(...lifecycleScriptFindings(readNearestPackageJson(entryDir)))
  if (findings.some((item) => item.level === 'error')) return { findings }

  const contents = packEntryContents(options.entry, options.sdkModule)
  const bundle = options.bundle ?? esbuildBundle
  const built = await bundle({ contents, resolveDir })
  findings.push(...bundleDependencyFindings(built.external))
  if (findings.some((item) => item.level === 'error')) return { findings }

  const outDir = resolve(options.outDir ?? process.cwd())
  await mkdir(outDir, { recursive: true })
  const file = join(outDir, packFileName(connector))
  const staging = await mkdtemp(join(tmpdir(), 'vorn-pack-'))
  try {
    await writeFile(join(staging, 'index.js'), built.code, 'utf8')
    await writeFile(
      join(staging, 'manifest.json'),
      `${JSON.stringify(connectorManifest(connector), null, 2)}\n`,
      'utf8'
    )
    const { create } = await import('tar')
    await create({ gzip: true, file, cwd: staging }, ['manifest.json', 'index.js'])
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
