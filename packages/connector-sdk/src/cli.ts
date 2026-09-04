#!/usr/bin/env node
import { fileURLToPath, pathToFileURL } from 'node:url'
import { existsSync, realpathSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { mkdir, writeFile } from 'node:fs/promises'
import { formatFindings, runConformance } from './check'
import { resolveConfig } from './define'
import { packConnector } from './pack'
import { esbuildBundle, type BundleOutput, type BundleRequest } from './packaging'
import { runPoll } from './runtime'
import { scaffoldFiles } from './scaffold'
import { connectionSetup, connectorManifest } from './setup'
import { serveConnector } from './server'
import type { Connector } from './types'

const USAGE = `vorn-connector <command> <module | id> [options]

Commands:
  new <id>                      Scaffold a new connector, ready to build
  manifest <module>             Print the connector manifest as JSON
  setup <module> [trigger]      Print the Vorn connection settings to paste
  check <module>                Verify the connector against Vorn's contract
  pack <module>                 Build an installable .vorn.tgz pack
  poll <module> <trigger>       Run one poll against the current environment
  serve <module>                Serve the connector on stdio (what Vorn runs)

Options:
  --since <iso>                 Lower bound passed to poll
  --limit <n>                   Maximum items to request
  --live                        Let check poll for real using the environment
  --mock                        Run every action against served HTTP, not the network
  --receipt <file>              Where check writes what it verified, as JSON
  --out <dir>                   Directory new and pack write to
  --name <name>                 Display name for a new connector
  --repo-conventions            Scaffold a package shaped for the connectors repository`

export interface CliDeps {
  load(modulePath: string): Promise<unknown>
  write(line: string): void
  env?: NodeJS.ProcessEnv
  /** Directory module paths resolve from; defaults to the working directory. */
  cwd?: string
  /** Replaced in tests so pack does not shell out to a bundler. */
  bundle?(request: BundleRequest): Promise<BundleOutput>
  /** Writes a scaffold file or a receipt; replaced in tests so nothing touches disk. */
  writeFile?(path: string, contents: string): Promise<void>
  /** Replaced in tests beside writeFile. */
  exists?(path: string): boolean
}

/** Flags that stand alone; everything else must be followed by a value. */
const BOOLEAN_FLAGS = new Set(['live', 'mock', 'repo-conventions'])

/**
 * Split arguments into flags and positionals in one pass, so a flag's value is
 * never also read as a positional argument.
 */
function parseArgs(args: string[]): {
  flags: Record<string, string>
  positional: string[]
} {
  const flags: Record<string, string> = {}
  const positional: string[] = []
  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (!arg.startsWith('--')) {
      positional.push(arg)
      continue
    }
    const name = arg.slice(2)
    if (BOOLEAN_FLAGS.has(name)) {
      flags[name] = 'true'
      continue
    }
    const value = args[index + 1]
    if (value === undefined || value.startsWith('--')) {
      throw new Error(`Missing value for ${arg}`)
    }
    flags[name] = value
    index++
  }
  return { flags, positional }
}

/** Accept either `export default connector` or `export const connector`. */
function pickConnector(loaded: unknown, modulePath: string): Connector {
  const module = loaded as Record<string, unknown> | null
  const candidate = module?.default ?? module?.connector
  const connector = candidate as Connector | undefined
  if (!connector || typeof connector !== 'object' || !Array.isArray(connector.triggers)) {
    throw new Error(
      `${modulePath} does not export a connector built with defineConnector() (default or named "connector")`
    )
  }
  return connector
}

export async function runCli(argv: string[], deps: CliDeps): Promise<number> {
  const [command, modulePath, ...rest] = argv
  if (!command || command === 'help' || command === '--help') {
    deps.write(USAGE)
    return command ? 0 : 1
  }
  if (!modulePath) {
    deps.write(`Missing <${command === 'new' ? 'id' : 'module'}> argument\n\n${USAGE}`)
    return 1
  }

  // Handled before the module is loaded: there is nothing to load yet.
  if (command === 'new') {
    const { flags } = parseArgs(rest)
    if (!deps.writeFile) {
      deps.write('This build cannot write files')
      return 1
    }
    const files = scaffoldFiles({
      id: modulePath,
      ...(flags.name !== undefined && { name: flags.name }),
      ...(flags['repo-conventions'] === 'true' && { repoConventions: true })
    })
    const root = join(flags.out ?? deps.cwd ?? '.', modulePath)
    if ((deps.exists ?? existsSync)(root)) {
      deps.write(`${root} already exists; a scaffold never overwrites`)
      return 1
    }
    for (const file of files) {
      await deps.writeFile(join(root, file.path), file.contents)
    }
    deps.write(`Created ${modulePath} in ${root}`)
    for (const file of files) deps.write(`  ${file.path}`)
    deps.write(`\nNext: cd ${root} && yarn install && yarn check`)
    return 0
  }

  const connector = pickConnector(await deps.load(modulePath), modulePath)
  const { flags, positional } = parseArgs(rest)

  switch (command) {
    case 'manifest':
      deps.write(JSON.stringify(connectorManifest(connector), null, 2))
      return 0

    case 'setup': {
      const triggers = positional[0] ? [positional[0]] : connector.triggers.map((t) => t.type)
      for (const triggerType of triggers) {
        const setup = connectionSetup(connector, triggerType)
        deps.write(`# ${connector.name} — ${triggerType}`)
        deps.write(`Command: npx`)
        deps.write(`Arguments: ["-y", "<your-package>"]`)
        deps.write(`Filters: ${JSON.stringify(setup.filters, null, 2)}`)
        if (setup.env.length > 0) {
          deps.write(
            `Environment: ${setup.env
              .map((entry) => `${entry.name}${entry.required ? ' (required)' : ''}`)
              .join(', ')}`
          )
        }
      }
      return 0
    }

    case 'check': {
      // A mock run is the whole conformance gate, so it also asks what the
      // connector ships as — the questions pack would ask, before packing.
      const packaged =
        flags.mock === 'true'
          ? {
              mock: true,
              packageDir: deps.cwd ?? process.cwd(),
              entry: modulePath,
              bundle: deps.bundle ?? esbuildBundle
            }
          : {}
      const run = await runConformance(connector, {
        ...packaged,
        ...(flags.live === 'true' && {
          live: true,
          config: resolveConfig(connector, deps.env ?? process.env)
        })
      })
      const { findings } = run
      const errors = findings.filter((item) => item.level === 'error')
      if (findings.length > 0) deps.write(formatFindings(findings))
      if (flags.receipt !== undefined) {
        if (run.receipt) {
          const write = deps.writeFile ?? ((path, contents) => writeFile(path, contents))
          await write(flags.receipt, `${JSON.stringify(run.receipt, null, 2)}\n`)
          deps.write(`Verified ${run.receipt.checks.join(', ')} — wrote ${flags.receipt}`)
        } else {
          deps.write(`No receipt written: nothing could be vouched for`)
          if (errors.length === 0) return 1
        }
      }
      deps.write(
        errors.length > 0
          ? `\n${errors.length} error(s), ${findings.length - errors.length} warning(s)`
          : `\n${connector.id} passed with ${findings.length} warning(s)`
      )
      return errors.length > 0 ? 1 : 0
    }

    case 'pack': {
      const result = await packConnector(connector, {
        entry: modulePath,
        ...(flags.out !== undefined && { outDir: flags.out }),
        ...(deps.cwd !== undefined && { resolveDir: deps.cwd }),
        ...(deps.bundle !== undefined && { bundle: deps.bundle })
      })
      if (result.findings.length > 0) deps.write(formatFindings(result.findings))
      const errors = result.findings.filter((item) => item.level === 'error')
      if (!result.file) {
        deps.write(`\n${errors.length} error(s) — nothing was packed`)
        return 1
      }
      deps.write(
        `\nPacked ${connector.id} ${connector.version} to ${result.file} (${Math.max(1, Math.round((result.bytes ?? 0) / 1024))} KB)`
      )
      return 0
    }

    case 'poll': {
      const triggerType = positional[0]
      if (!triggerType) {
        deps.write(`Missing <trigger> argument\n\n${USAGE}`)
        return 1
      }
      const limit = flags.limit === undefined ? undefined : Number(flags.limit)
      if (limit !== undefined && !Number.isFinite(limit)) {
        deps.write(`Invalid limit "${flags.limit}"`)
        return 1
      }
      const page = await runPoll(connector, triggerType, {
        config: resolveConfig(connector, deps.env ?? process.env),
        ...(flags.since !== undefined && { since: flags.since }),
        ...(limit !== undefined && { limit })
      })
      deps.write(JSON.stringify(page, null, 2))
      return 0
    }

    case 'serve':
      await serveConnector(connector)
      return 0

    default:
      deps.write(`Unknown command "${command}"\n\n${USAGE}`)
      return 1
  }
}

/* c8 ignore start -- process wiring exercised by the bin, not by unit tests */
// Compared through realpath: a .bin launcher or a portal reaches this file through a symlink.
export function isEntryPoint(moduleUrl: string, argv: readonly string[] = process.argv): boolean {
  const invoked = argv[1]
  if (invoked === undefined) return false
  try {
    return realpathSync(fileURLToPath(moduleUrl)) === realpathSync(resolve(invoked))
  } catch {
    return false
  }
}
const invokedDirectly = isEntryPoint(import.meta.url)

if (invokedDirectly) {
  runCli(process.argv.slice(2), {
    load: (modulePath) =>
      import(
        modulePath.startsWith('.') || modulePath.startsWith('/')
          ? pathToFileURL(resolve(modulePath)).href
          : modulePath
      ),
    write: (line) => process.stdout.write(`${line}\n`),
    writeFile: async (path, contents) => {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, contents)
    }
  })
    .then((code) => {
      process.exitCode = code
    })
    .catch((error: unknown) => {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
      process.exitCode = 1
    })
}
/* c8 ignore stop */
