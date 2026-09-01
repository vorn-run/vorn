import { builtinModules } from 'node:module'
import { mkdtemp, mkdir, rm, stat, writeFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { checkConnector, type CheckFinding } from './check'
import { connectorManifest } from './setup'
import type { Connector } from './types'

/** Largest pack Vorn will install, matched by the server's own verification. */
export const MAX_PACK_BYTES = 8 * 1024 * 1024

/** Scripts npm would run at install time, which a pack must never carry. */
const LIFECYCLE_SCRIPTS = [
  'preinstall',
  'install',
  'postinstall',
  'prepare',
  'prepublish',
  'prepublishOnly',
  'postpublish'
]

const BUILTINS = new Set(builtinModules)

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

export interface BundleRequest {
  contents: string
  resolveDir: string
}

export interface BundleOutput {
  code: string
  /** Specifiers the bundler left for the runtime to resolve. */
  external: string[]
}

export interface PackResult {
  findings: CheckFinding[]
  /** Absolute path of the written pack; absent when a gate failed. */
  file?: string
  bytes?: number
}

function finding(code: string, target: string, message: string): CheckFinding {
  return { level: 'error', code, target, message }
}

/** Reject a source package whose install would run code on the user's machine. */
export function lifecycleScriptFindings(pkg: unknown): CheckFinding[] {
  const scripts = (pkg as { scripts?: Record<string, unknown> } | null)?.scripts
  if (!scripts || typeof scripts !== 'object') return []
  const named = LIFECYCLE_SCRIPTS.filter((name) => typeof scripts[name] === 'string')
  if (named.length === 0) return []
  return [
    finding(
      'lifecycle-scripts',
      'package.json',
      `Remove the ${named.join(', ')} script(s); a pack is installed by copying files, never by running them`
    )
  ]
}

/** Specifiers left outside a bundle, which would need a registry at launch. */
export function bundleDependencyFindings(external: string[]): CheckFinding[] {
  const specifiers = new Set<string>()
  for (const specifier of external) {
    if (specifier.startsWith('.') || specifier.startsWith('/')) continue
    if (specifier.startsWith('node:') || BUILTINS.has(specifier)) continue
    specifiers.add(specifier)
  }
  if (specifiers.size === 0) return []
  return [
    finding(
      'runtime-dependencies',
      'bundle',
      `${[...specifiers].sort().join(', ')} stayed outside the bundle; a pack must launch with no install step`
    )
  ]
}

/** Nearest package.json at or above a directory, or undefined when there is none. */
export function readNearestPackageJson(fromDir: string): Record<string, unknown> | undefined {
  let current = resolve(fromDir)
  for (;;) {
    try {
      return JSON.parse(readFileSync(join(current, 'package.json'), 'utf8')) as Record<
        string,
        unknown
      >
    } catch {
      const parent = dirname(current)
      if (parent === current) return undefined
      current = parent
    }
  }
}

async function esbuildBundle(request: BundleRequest): Promise<BundleOutput> {
  const { build } = await import('esbuild')
  const result = await build({
    stdin: {
      contents: request.contents,
      resolveDir: request.resolveDir,
      sourcefile: 'vorn-connector-pack.js',
      loader: 'js'
    },
    bundle: true,
    platform: 'node',
    target: 'node20',
    format: 'esm',
    write: false,
    metafile: true,
    legalComments: 'none'
  })
  const output = Object.values(result.metafile.outputs)[0]
  return {
    code: result.outputFiles[0].text,
    external: (output?.imports ?? []).filter((item) => item.external).map((item) => item.path)
  }
}

/** File name Vorn recognizes as a connector pack. */
export function packFileName(connector: Connector): string {
  return `${connector.id}-${connector.version}.vorn.tgz`
}

/**
 * Build the installable pack: a gzipped tar of the manifest and one bundled
 * stdio entry. The entry is generated rather than taken from the connector's
 * own bin so every pack launches the same way, whatever its author wrote.
 */
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

  const sdkModule = options.sdkModule ?? '@vornrun/connector-sdk'
  const contents = [
    `import { serveConnector } from ${JSON.stringify(sdkModule)}`,
    `import * as entry from ${JSON.stringify(options.entry)}`,
    'const exported = Object.values(entry).find((value) => value && Array.isArray(value.triggers))',
    `if (!exported) throw new Error(${JSON.stringify(`${options.entry} exports no connector`)})`,
    'await serveConnector(exported)',
    ''
  ].join('\n')
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
