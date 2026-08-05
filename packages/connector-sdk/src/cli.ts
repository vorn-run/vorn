#!/usr/bin/env node
import { pathToFileURL } from 'node:url'
import { resolve } from 'node:path'
import { checkConnector, formatFindings } from './check'
import { resolveConfig } from './define'
import { runPoll } from './runtime'
import { connectionSetup, connectorManifest } from './setup'
import { serveConnector } from './server'
import type { Connector } from './types'

const USAGE = `vorn-connector <command> <module> [options]

Commands:
  manifest <module>             Print the connector manifest as JSON
  setup <module> [trigger]      Print the Vorn connection settings to paste
  check <module>                Verify the connector against Vorn's contract
  poll <module> <trigger>       Run one poll against the current environment
  serve <module>                Serve the connector on stdio (what Vorn runs)

Options:
  --since <iso>                 Lower bound passed to poll
  --limit <n>                   Maximum items to request
  --live                        Let check poll for real using the environment`

export interface CliDeps {
  load(modulePath: string): Promise<unknown>
  write(line: string): void
  env?: NodeJS.ProcessEnv
}

/** Flags that stand alone; everything else must be followed by a value. */
const BOOLEAN_FLAGS = new Set(['live'])

function parseFlags(args: string[]): Record<string, string> {
  const flags: Record<string, string> = {}
  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (!arg.startsWith('--')) continue
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
  return flags
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
    deps.write(`Missing <module> argument\n\n${USAGE}`)
    return 1
  }

  const connector = pickConnector(await deps.load(modulePath), modulePath)
  const positional = rest.filter((arg) => !arg.startsWith('--'))
  const flags = parseFlags(rest)

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
      const findings = await checkConnector(connector, {
        ...(flags.live === 'true' && {
          live: true,
          config: resolveConfig(connector, deps.env ?? process.env)
        })
      })
      const errors = findings.filter((item) => item.level === 'error')
      if (findings.length > 0) deps.write(formatFindings(findings))
      deps.write(
        errors.length > 0
          ? `\n${errors.length} error(s), ${findings.length - errors.length} warning(s)`
          : `\n${connector.id} passed with ${findings.length} warning(s)`
      )
      return errors.length > 0 ? 1 : 0
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
const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href

if (invokedDirectly) {
  runCli(process.argv.slice(2), {
    load: (modulePath) =>
      import(
        modulePath.startsWith('.') || modulePath.startsWith('/')
          ? pathToFileURL(resolve(modulePath)).href
          : modulePath
      ),
    write: (line) => process.stdout.write(`${line}\n`)
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
