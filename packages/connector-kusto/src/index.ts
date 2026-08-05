import { createRequire } from 'node:module'
import { realpathSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { serveConnector } from '@vornrun/connector-sdk'
import { createKustoConnector } from './connector'

export { createKustoConnector } from './connector'
export type { KustoConnectorOptions } from './connector'

const { version } = createRequire(import.meta.url)('../package.json') as { version: string }

export const kustoConnector = createKustoConnector({ version })

/**
 * True when this module is the process entry point.
 *
 * Compared through `realpathSync` because Vorn launches the connector via the
 * `node_modules/.bin` symlink, where `argv[1]` is the link and
 * `import.meta.url` is its target.
 */
function isEntryPoint(): boolean {
  const entry = process.argv[1]
  if (!entry) return false
  try {
    return realpathSync(entry) === realpathSync(fileURLToPath(import.meta.url))
  } catch {
    return false
  }
}

// Importing the module — tests, `vorn-connector check` — must start nothing.
if (isEntryPoint()) void serveConnector(kustoConnector)
