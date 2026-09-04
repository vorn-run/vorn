import { builtinModules } from 'node:module'
import { readFileSync } from 'node:fs'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import type { CheckCode, CheckFinding } from './check'
import { connectorManifest } from './setup'
import type { Connector } from './types'

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

// `require('../x')`, esbuild's `__require('../x')`, `require.resolve('../x')` and `import('../x')`.
const RELATIVE_REQUIRE =
  /(?:^|[^\w$.])(?:__)?(?:require(?:\.resolve)?|import)\(\s*(['"])(\.\.?\/[^'"]*)\1\s*\)/g

// The `createRequire(...)('../x')` form, but not the bare helper esbuild emits and never calls with a path.
const RELATIVE_CREATE_REQUIRE = /createRequire\([^()]*\)\(\s*(['"])(\.\.?\/[^'"]*)\1\s*\)/g

/** Files a pack does not carry, asked for after it is installed. */
export function bundledRequireFindings(code: string): CheckFinding[] {
  const specifiers = new Set<string>()
  for (const pattern of [RELATIVE_REQUIRE, RELATIVE_CREATE_REQUIRE]) {
    for (const [, , specifier] of code.matchAll(pattern)) specifiers.add(specifier)
  }
  if (specifiers.size === 0) return []
  return [
    finding(
      'runtime-dependencies',
      'bundle',
      `${[...specifiers].sort().join(', ')} is required at runtime; a pack is one file, so nothing beside it survives packing`
    )
  ]
}

/**
 * The directory whose package.json describes the connector being examined.
 *
 * A path-like entry names its own package — checking `packages/slack/dist` from
 * a monorepo root has to judge Slack's package.json, not the root's — while a
 * bare specifier was resolved from the working directory, so that is what
 * describes it. `check` and `pack` share this, or a gate would judge one
 * package and the artifact come from another.
 */
export function packageDirFor(resolveDir: string, entry: string | undefined): string {
  const from = resolve(resolveDir)
  if (entry === undefined) return from
  return entry.startsWith('.') || isAbsolute(entry) ? dirname(resolve(from, entry)) : from
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

/** A directory holding nothing but the two files a pack carries, for tarring or for launching. */
export async function stagePack(connector: Connector, code: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'vorn-pack-'))
  await writeFile(join(dir, 'index.js'), code, 'utf8')
  await writeFile(
    join(dir, 'manifest.json'),
    `${JSON.stringify(connectorManifest(connector), null, 2)}\n`,
    'utf8'
  )
  return dir
}

/** Long enough for a cold start on a loaded machine, short enough to fail a hung one. */
const LAUNCH_TIMEOUT_MS = 15_000

// Beyond the transport's own allowlist, which omits these; an ambient token still never reaches the child.
const LAUNCH_ENV_KEYS = ['TMPDIR', 'TEMP', 'TMP', 'LANG', 'LC_ALL', 'TZ', 'PATHEXT', 'COMSPEC']

function launchEnv(): Record<string, string> {
  const env: Record<string, string> = {}
  for (const key of LAUNCH_ENV_KEYS) {
    const value = process.env[key]
    if (value !== undefined) env[key] = value
  }
  return env
}

/** The line naming the failure, which node prints below the frames and the banners a connector logs first. */
function errorLine(text: string): string | undefined {
  const lines = text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '')
  return [...lines].reverse().find((line) => /Error\b/.test(line)) ?? lines[lines.length - 1]
}

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: NodeJS.Timeout
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms)
    })
  ])
}

// Start a staged pack the way the host does: only a completed `initialize` counts, so a bundle that logs and exits cannot pass.
export async function packLaunchFindings(dir: string): Promise<CheckFinding[]> {
  const { Client } = await import('@modelcontextprotocol/sdk/client/index.js')
  const { StdioClientTransport } = await import('@modelcontextprotocol/sdk/client/stdio.js')
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: ['index.js'],
    cwd: dir,
    env: launchEnv(),
    stderr: 'pipe'
  })
  const client = new Client({ name: 'vorn-connector-check', version: '1' }, { capabilities: {} })
  let stderr = ''
  // A PassThrough is handed over before the spawn, so nothing the child says on its way out is missed.
  transport.stderr?.on('data', (chunk: Buffer) => {
    stderr += chunk.toString()
  })
  try {
    await withTimeout(
      client.connect(transport),
      LAUNCH_TIMEOUT_MS,
      `did not answer within ${LAUNCH_TIMEOUT_MS / 1000}s of starting`
    )
    return []
  } catch (error) {
    const said = error instanceof Error ? error.message : String(error)
    return [
      finding('pack-launch', 'bundle', `did not start as a pack: ${errorLine(stderr) ?? said}`)
    ]
  } finally {
    // The child must not outlive the check, including when the timeout fired while it still ran.
    await client.close().catch(() => {})
    await transport.close().catch(() => {})
  }
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
