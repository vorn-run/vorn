/**
 * The files a new connector starts as.
 *
 * A connector is mostly boilerplate — a package that builds, an entry that
 * serves, a definition, a test that proves it without a network — and getting
 * that boilerplate right is the slowest part of writing the interesting bit.
 * Generating it means every connector starts from the same shape, which is
 * also the shape `check` and `pack` expect to find.
 *
 * The files are returned rather than written so the decision of what to write
 * stays testable, and the writing stays in the CLI.
 */

const ID_PATTERN = /^[a-zA-Z][a-zA-Z0-9_-]*$/

/**
 * What a scaffolded connector depends on.
 *
 * Names a prerelease on purpose. Only `0.7.0-beta.x` is published, and a bare
 * `^0.7.0` matches no prerelease at all — a scaffold pinned to it installs
 * nothing. Bumped with the SDK's own version until a stable one exists.
 */
const SDK_DEPENDENCY_RANGE = '^0.7.0-beta.8'

/** What a scaffold starts at, in the package and in the changelog section that must match it. */
const SCAFFOLD_VERSION = '0.1.0'

export interface ScaffoldOptions {
  id: string
  /** Defaults to the id in title case. */
  name?: string
  description?: string
  /** Emit the shape the connectors repository expects of a package inside it. */
  repoConventions?: boolean
}

export interface ScaffoldFile {
  /** Relative to the directory the connector is created in. */
  path: string
  contents: string
}

/** `acme-tickets` → `Acme Tickets`, so a generated name reads like a name. */
export function titleCase(id: string): string {
  return id
    .split(/[-_]+/)
    .filter((part) => part !== '')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
}

function packageJson(id: string, description: string, inRepo: boolean): string {
  return `${JSON.stringify(
    {
      name: inRepo ? `@vornrun/connector-${id}` : `vorn-connector-${id}`,
      version: SCAFFOLD_VERSION,
      description,
      type: 'module',
      license: 'MIT',
      bin: { [`vorn-connector-${id}`]: 'dist/index.js' },
      main: './dist/index.js',
      types: './dist/index.d.ts',
      files: inRepo ? ['dist', 'README.md', 'CHANGELOG.md'] : ['dist', 'README.md'],
      ...(inRepo && {
        repository: {
          type: 'git',
          url: 'git+https://github.com/vorn-run/connectors.git',
          directory: `packages/${id}`
        }
      }),
      scripts: {
        build: 'tsup src/index.ts --format esm --target node22 --clean --dts',
        check: 'vorn-connector check src/index.ts',
        pack: 'vorn-connector pack src/index.ts',
        test: 'vitest run',
        typecheck: 'tsc --noEmit'
      },
      dependencies: { '@vornrun/connector-sdk': SDK_DEPENDENCY_RANGE },
      devDependencies: {
        ...(inRepo && { '@types/node': '^22.10.2', '@vitest/coverage-v8': '^4.1.10' }),
        tsup: '^8.5.1',
        typescript: '^6.0.3',
        vitest: '^4.1.10'
      },
      // Read by the catalog build: how this connector is filed, found, and what it asks of you.
      vorn: {
        category: 'Other',
        keywords: [id],
        ...(inRepo && { auth: 'Say in one line what signing in takes.' })
      }
    },
    null,
    2
  )}\n`
}

function tsconfig(): string {
  return `${JSON.stringify(
    {
      compilerOptions: {
        target: 'ES2022',
        lib: ['ES2022'],
        module: 'ESNext',
        moduleResolution: 'bundler',
        allowSyntheticDefaultImports: true,
        esModuleInterop: true,
        strict: true,
        skipLibCheck: true,
        types: ['node'],
        noEmit: true,
        ignoreDeprecations: '6.0',
        allowImportingTsExtensions: true
      },
      include: ['src/**/*', 'vitest.config.ts']
    },
    null,
    2
  )}\n`
}

function tsupConfig(): string {
  return `import { defineConfig } from 'tsup'

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  target: 'node22',
  clean: true,
  dts: true,
  // Vorn spawns the built file directly.
  banner: { js: '#!/usr/bin/env node' },
  external: ['@modelcontextprotocol/sdk', '@vornrun/connector-sdk', 'zod']
})
`
}

function vitestConfig(): string {
  return `// The repository's one test configuration, so no package drifts from its coverage gate.
export { default } from '../../vitest.shared.ts'
`
}

function changelog(): string {
  return `# Changelog

## ${SCAFFOLD_VERSION}

- First release.
`
}

function connectorSource(id: string, name: string, description: string): string {
  return `import { defineConnector } from '@vornrun/connector-sdk'

export const connector = defineConnector({
  id: ${JSON.stringify(id)},
  name: ${JSON.stringify(name)},
  description: ${JSON.stringify(description)},
  version: '0.1.0',
  // Prefer a login the machine already has: { rung: 'cli', probe: { command: 'tool', args: ['auth', 'status'] } }
  auth: { rung: 'key', keys: ['apiToken'] },
  config: [
    {
      key: 'apiToken',
      label: 'API token',
      required: true,
      secret: true,
      builderHint: 'Say where a token is created and which scopes it needs'
    },
    { key: 'baseUrl', label: 'Base URL', default: 'https://api.example.com' }
  ],
  triggers: [
    {
      type: 'itemCreated',
      label: 'Item created',
      description: 'Items created since the last poll',
      // Return what is there; the SDK handles cursors and de-duplication.
      dedupe: 'timestamp',
      async fetch(context) {
        const url = new URL('/v1/items', context.config.baseUrl)
        if (context.since) url.searchParams.set('updated_since', context.since)
        // \`context.fetch\` retries and backs off; the global one does not.
        const response = await context.fetch(url, {
          headers: { authorization: 'Bearer ' + context.config.apiToken }
        })
        if (!response.ok) throw new Error('Listing items failed with ' + response.status)
        const body = (await response.json()) as { items: Array<Record<string, string>> }
        return body.items.map((item) => ({
          externalId: item.id,
          title: item.title,
          url: item.html_url,
          updatedAt: item.updated_at
        }))
      }
    }
  ],
  actions: [
    {
      type: 'createItem',
      label: 'Create item',
      description: 'Create one item',
      inputs: [
        { key: 'title', label: 'Title', required: true },
        { key: 'body', label: 'Body' }
      ],
      outputs: [{ key: 'id', type: 'string', description: 'The created item' }],
      // Declared, not written: the SDK fills the templates, sends it, and keeps what postReceive names.
      request: {
        method: 'POST',
        url: '{{config.baseUrl}}/v1/items',
        headers: { authorization: 'Bearer {{config.apiToken}}' },
        body: { title: '{{args.title}}', body: '{{args.body}}' }
      },
      postReceive: [{ op: 'pick', keys: ['id'] }]
    }
  ]
})
`
}

function entrySource(): string {
  return `import { realpathSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { serveConnector } from '@vornrun/connector-sdk'
import { connector } from './connector'

/** True when this file was run directly rather than imported. */
export function isEntryPoint(moduleUrl: string, argv = process.argv): boolean {
  const invoked = argv[1]
  if (invoked === undefined) return false
  try {
    return realpathSync(fileURLToPath(moduleUrl)) === realpathSync(invoked)
  } catch {
    return false
  }
}

/** Serve on stdio when run directly. This is what Vorn spawns. */
export async function serveIfEntryPoint(moduleUrl: string): Promise<void> {
  if (isEntryPoint(moduleUrl)) await serveConnector(connector)
}
`
}

function indexSource(): string {
  return `import { connector } from './connector'
import { serveIfEntryPoint } from './entry'

export { connector }
export default connector

await serveIfEntryPoint(import.meta.url)
`
}

function testSource(name: string): string {
  return `import { describe, expect, it, vi } from 'vitest'
import { createConnectorHarness } from '@vornrun/connector-sdk'
import { connector } from './connector'

/** Answers the connector's calls from here, so the test needs no network. */
function fakeFetch(body: unknown) {
  return vi.fn(async () =>
    new Response(JSON.stringify(body), { headers: { 'content-type': 'application/json' } })
  ) as unknown as typeof fetch
}

const config = { apiToken: 'test-token', baseUrl: 'https://api.example.com' }

describe(${JSON.stringify(name)}, () => {
  it('reports the items the source lists', async () => {
    const harness = createConnectorHarness(connector, {
      config,
      fetchImpl: fakeFetch({
        items: [
          {
            id: '1',
            title: 'First item',
            html_url: 'https://example.com/1',
            updated_at: '2026-01-01T00:00:00.000Z'
          }
        ]
      })
    })

    const page = await harness.poll('itemCreated')

    expect(page.items).toHaveLength(1)
    expect(page.items[0].externalId).toBe('1')
  })

  it('does not deliver the same item twice', async () => {
    const harness = createConnectorHarness(connector, {
      config,
      fetchImpl: fakeFetch({
        items: [
          {
            id: '1',
            title: 'First item',
            html_url: 'https://example.com/1',
            updated_at: '2026-01-01T00:00:00.000Z'
          }
        ]
      })
    })

    expect(await harness.pollTwice('itemCreated')).toEqual([])
  })

  it('creates an item and keeps only its id', async () => {
    const harness = createConnectorHarness(connector, {
      config,
      fetchImpl: fakeFetch({ id: '42', extra: 'ignored' })
    })

    expect(await harness.execute('createItem', { title: 'A title' })).toEqual({ id: '42' })
  })
})
`
}

function readme(id: string, name: string, description: string): string {
  return `# ${name}

${description}

## Build and check

\`\`\`sh
yarn install
yarn build
yarn check      # verifies the connector against Vorn's contract
yarn test
yarn pack       # writes ${id}-0.1.0.vorn.tgz, installable in Vorn
\`\`\`

## Settings

| Setting | Environment | Required |
| --- | --- | --- |
| API token | \`API_TOKEN\` | yes |
| Base URL | \`BASE_URL\` | no |

## What it offers

- **Item created** — polls for items created since the last run.
- **Create item** — creates one item and returns its id.

Rename the trigger, the action and the settings to whatever this connector
really talks to; the shapes here are a starting point, not a rule.
`
}

/** Every file a new connector starts with, ready to build, check and pack. */
export function scaffoldFiles(options: ScaffoldOptions): ScaffoldFile[] {
  if (!ID_PATTERN.test(options.id ?? '')) {
    throw new Error(`Connector id "${options.id}" must start with a letter and be url-safe`)
  }
  const name = options.name?.trim() || titleCase(options.id)
  const description = options.description?.trim() || `${name} connector for Vorn`
  const inRepo = options.repoConventions === true

  return [
    { path: 'package.json', contents: packageJson(options.id, description, inRepo) },
    { path: 'src/connector.ts', contents: connectorSource(options.id, name, description) },
    { path: 'src/entry.ts', contents: entrySource() },
    { path: 'src/index.ts', contents: indexSource() },
    { path: 'src/connector.test.ts', contents: testSource(name) },
    { path: 'README.md', contents: readme(options.id, name, description) },
    // The rest of what a package in the connectors repository has to carry.
    ...(inRepo
      ? [
          { path: 'CHANGELOG.md', contents: changelog() },
          { path: 'tsconfig.json', contents: tsconfig() },
          { path: 'tsup.config.ts', contents: tsupConfig() },
          { path: 'vitest.config.ts', contents: vitestConfig() }
        ]
      : [])
  ]
}
