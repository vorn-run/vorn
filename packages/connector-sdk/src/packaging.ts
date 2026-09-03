import { builtinModules } from 'node:module'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import type { CheckCode, CheckFinding } from './check'

/**
 * What a connector must be true of as a *package*, rather than as a definition.
 *
 * Both `check` and `pack` ask these questions — check to fail a pull request
 * early, pack to refuse an artifact — so they live here rather than in either,
 * and neither imports the other.
 */

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

export interface BundleRequest {
  contents: string
  resolveDir: string
}

export interface BundleOutput {
  code: string
  /** Specifiers the bundler left for the runtime to resolve. */
  external: string[]
}

function finding(code: CheckCode, target: string, message: string): CheckFinding {
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

/**
 * The stdio entry a pack is built from.
 *
 * `check` bundles exactly this too: a gate that asked a different question than
 * the one pack asks would pass a connector pack then refuses.
 */
export function packEntryContents(entry: string, sdkModule = '@vornrun/connector-sdk'): string {
  return [
    `import { serveConnector } from ${JSON.stringify(sdkModule)}`,
    `import * as entry from ${JSON.stringify(entry)}`,
    'const exported = Object.values(entry).find((value) => value && Array.isArray(value.triggers))',
    `if (!exported) throw new Error(${JSON.stringify(`${entry} exports no connector`)})`,
    'await serveConnector(exported)',
    ''
  ].join('\n')
}

/** The bundler `pack` uses, shared so `check` gates on the same answer. */
export async function esbuildBundle(request: BundleRequest): Promise<BundleOutput> {
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
