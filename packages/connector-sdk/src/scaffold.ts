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
const SDK_DEPENDENCY_RANGE = '^0.7.0-beta.14'

/** What a scaffold starts at, in the package and in the changelog section that must match it. */
const SCAFFOLD_VERSION = '0.1.0'
const VITEST_RANGE = '^4.1.10'

function jsonFile(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`
}

export interface ScaffoldOptions {
  id: string
  /** Defaults to the id in title case. */
  name?: string
  description?: string
  /** Emit the shape the connectors repository expects of a package inside it. */
  repoConventions?: boolean
  /** What to start: a connector that polls a service, or an extension that contributes to a card. */
  kind?: 'connector' | 'extension'
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

function packageJson(
  id: string,
  description: string,
  inRepo: boolean,
  kind: 'connector' | 'extension'
): string {
  const scoped = kind === 'extension' ? 'extension' : 'connector'
  return jsonFile({
    name: inRepo ? `@vornrun/${scoped}-${id}` : `vorn-${scoped}-${id}`,
    version: SCAFFOLD_VERSION,
    description,
    type: 'module',
    license: 'MIT',
    bin: { [`vorn-${scoped}-${id}`]: 'dist/index.js' },
    main: './dist/index.js',
    types: './dist/index.d.ts',
    // A pane's page ships beside the bundle, so `web` is published like `dist`.
    files: [
      'dist',
      'README.md',
      ...(kind === 'extension' ? ['web'] : []),
      ...(inRepo ? ['CHANGELOG.md'] : [])
    ],
    ...(inRepo && {
      repository: {
        type: 'git',
        url: 'git+https://github.com/vorn-run/connectors.git',
        directory: `packages/${id}`
      }
    }),
    scripts: {
      // In the repository the config file is the one definition of the build.
      build: inRepo ? 'tsup' : 'tsup src/index.ts --format esm --target node22 --clean --dts',
      check: 'vorn-connector check src/index.ts',
      pack: 'vorn-connector pack src/index.ts',
      test: 'vitest run',
      typecheck: 'tsc --noEmit'
    },
    dependencies: { '@vornrun/connector-sdk': SDK_DEPENDENCY_RANGE },
    devDependencies: {
      ...(inRepo && { '@types/node': '^22.10.2', '@vitest/coverage-v8': VITEST_RANGE }),
      tsup: '^8.5.1',
      typescript: '^6.0.3',
      vitest: VITEST_RANGE
    },
    // Read by the catalog build: how this is filed, found, and what it asks of you.
    vorn: {
      category: kind === 'extension' ? 'Extensions' : 'Other',
      keywords: [id],
      ...(kind === 'extension' && { kind: 'extension' }),
      ...(inRepo && kind === 'connector' && { auth: 'Say in one line what signing in takes.' })
    }
  })
}

function tsconfig(inRepo: boolean): string {
  return jsonFile({
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
      resolveJsonModule: true,
      ignoreDeprecations: '6.0',
      allowImportingTsExtensions: true
    },
    include: ['src/**/*', ...(inRepo ? ['vitest.config.ts'] : [])]
  })
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
  banner: { js: '#!/usr/bin/env node' }
})
`
}

function vitestConfig(): string {
  return `import shared from '../../vitest.shared.ts'

export default shared
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
// Bundled at build time: a pack is one file, so a version read from disk is not there to read.
import pkg from '../package.json'

export const connector = defineConnector({
  id: ${JSON.stringify(id)},
  name: ${JSON.stringify(name)},
  description: ${JSON.stringify(description)},
  version: pkg.version,
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

function extensionSource(id: string, name: string, description: string): string {
  return `import { defineExtension } from '@vornrun/connector-sdk'
// Bundled at build time: a pack is one file, so a version read from disk is not there to read.
import pkg from '../package.json'

export const connector = defineExtension({
  id: ${JSON.stringify(id)},
  name: ${JSON.stringify(name)},
  description: ${JSON.stringify(description)},
  version: pkg.version,
  // Only what this actually spends: the check names one it declared and never used.
  permissions: ['terminal.read'],
  // Absent where none of these hold, rather than showing a band with nothing in it.
  activates: { workspaceContains: ['package.json'] },
  footers: [
    {
      id: 'checks',
      title: 'Checks',
      description: 'What the last commands in this session said',
      every: 30,
      async run(context) {
        const output = await context.host.output({ lines: 200 })
        const failed = /\\b(FAIL|failed|error)\\b/i.test(output)
        return [
          {
            label: 'tests',
            value: failed ? 'failing' : 'passing',
            tone: failed ? 'danger' : 'ok'
          }
        ]
      }
    }
  ],
  panes: [
    {
      id: 'report',
      title: 'Report',
      description: 'The reading, in full, beside the terminal',
      // Served from the pack; everything the page needs lives under web/.
      web: 'web/report/index.html'
    }
  ]
})
`
}

function extensionPage(name: string): string {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>${name}</title>
    <style>
      body {
        margin: 0;
        padding: 12px;
        font: 12px ui-sans-serif, system-ui, sans-serif;
        color: #faf9f7;
        background: #101012;
      }
      h1 {
        font-size: 13px;
        font-weight: 500;
        margin: 0 0 8px;
      }
      pre {
        margin: 0;
        white-space: pre-wrap;
        color: rgba(255, 255, 255, 0.55);
      }
    </style>
  </head>
  <body>
    <h1>${name}</h1>
    <pre id="output">Reading the session…</pre>
    <script type="module">
      // Same origin, so the page carries no credential: Vorn knows which pane is
      // asking and grants exactly the permissions the manifest declared.
      const ask = async (method, body = {}) => {
        const response = await fetch('bridge/' + method, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(body)
        })
        if (!response.ok) throw new Error(method + ' answered ' + response.status)
        return (await response.json()).result
      }

      const node = document.getElementById('output')
      try {
        node.textContent = await ask('output', { lines: 200 })
      } catch (error) {
        node.textContent = String(error)
      }
    </script>
  </body>
</html>
`
}

function extensionTestSource(name: string): string {
  return `import { describe, expect, it } from 'vitest'
import { mockExtensionHost } from '@vornrun/connector-sdk'
import { connector } from './extension'

/** Runs one footer the way the host will, against a stub that enforces the manifest. */
async function footer(id: string, output: string) {
  const { host } = mockExtensionHost(connector.permissions ?? [], { output: async () => output })
  const declared = connector.contributes?.footers?.find((entry) => entry.id === id)
  if (!declared) throw new Error('no footer ' + id)
  return declared.run({
    sessionId: 'test',
    worktreePath: process.cwd(),
    agent: 'claude',
    host,
    now: () => '2026-01-01T00:00:00.000Z'
  })
}

describe(${JSON.stringify(name)}, () => {
  it('reads the session as passing when nothing failed', async () => {
    expect(await footer('checks', 'Test Files  1 passed (1)')).toEqual([
      { label: 'tests', value: 'passing', tone: 'ok' }
    ])
  })

  it('reads it as failing when the output says so', async () => {
    expect(await footer('checks', 'FAIL src/index.test.ts')).toEqual([
      { label: 'tests', value: 'failing', tone: 'danger' }
    ])
  })

  it('asks for nothing it did not declare', () => {
    expect(connector.permissions).toEqual(['terminal.read'])
  })
})
`
}

function entrySource(module: string): string {
  return `import { realpathSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { serveConnector } from '@vornrun/connector-sdk'
import { connector } from './${module}'

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

/** Serve on stdio when run directly, which is what Vorn spawns; says whether it did. */
export async function serveIfEntryPoint(
  moduleUrl: string,
  serve: (c: typeof connector) => Promise<void> = serveConnector
): Promise<boolean> {
  if (!isEntryPoint(moduleUrl)) return false
  await serve(connector)
  return true
}
`
}

function entryTestSource(): string {
  return `import { describe, expect, it, vi } from 'vitest'
import { realpathSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { isEntryPoint, serveIfEntryPoint } from './entry'
import connector, { connector as named } from './index'

const HERE = import.meta.url

describe('isEntryPoint', () => {
  it('is false when the process was started without a script', () => {
    expect(isEntryPoint(HERE, ['node'])).toBe(false)
  })

  it('is true when argv points at this module, through a symlink or not', () => {
    expect(isEntryPoint(HERE, ['node', fileURLToPath(HERE)])).toBe(true)
    expect(isEntryPoint(HERE, ['node', realpathSync(fileURLToPath(HERE))])).toBe(true)
  })

  it('is false when the module is running under the test runner', () => {
    expect(isEntryPoint(HERE)).toBe(false)
  })

  it('is false rather than throwing when a path cannot be resolved', () => {
    expect(isEntryPoint(HERE, ['node', '/nowhere/that/exists'])).toBe(false)
  })
})

describe('serveIfEntryPoint', () => {
  it('starts nothing when the module was merely imported', async () => {
    const serve = vi.fn(async () => {})
    expect(await serveIfEntryPoint(HERE, serve)).toBe(false)
    expect(serve).not.toHaveBeenCalled()
  })
})

describe('the packaged connector', () => {
  it('is the same connector under both exports', () => {
    expect(connector).toBe(named)
    expect(connector.version).toMatch(/^\\d+\\.\\d+\\.\\d+/)
  })
})
`
}

function indexSource(module: string): string {
  return `import { connector } from './${module}'
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
yarn pack       # writes ${id}-${SCAFFOLD_VERSION}.vorn.tgz, installable in Vorn
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

function extensionReadme(id: string, name: string, description: string): string {
  return `# ${name}

${description}

## Build and check

\`\`\`sh
yarn install
yarn build
yarn check      # verifies the extension against Vorn's contract
yarn test
yarn pack       # writes ${id}-${SCAFFOLD_VERSION}.vorn.tgz, installable in Vorn
\`\`\`

## What it contributes

| Kind | Name | What it does |
| --- | --- | --- |
| Footer | Checks | A band under the card's status bar, recomputed every 30s |
| Pane | Report | A page beside the terminal, served from \`web/report\` |

## What it asks for

| Permission | What it grants |
| --- | --- |
| \`terminal.read\` | The session's recent terminal output |

Ask for only what the extension spends: \`check\` names a permission that was
declared and never used.

## Where it shows

Sessions whose worktree has a \`package.json\`. Widen or narrow that in
\`activates\`, and narrow one contribution further with its own \`when\`.
`
}

/** Every file a new connector or extension starts with, ready to build, check and pack. */
export function scaffoldFiles(options: ScaffoldOptions): ScaffoldFile[] {
  const kind = options.kind ?? 'connector'
  if (!ID_PATTERN.test(options.id ?? '')) {
    throw new Error(
      `${kind === 'extension' ? 'Extension' : 'Connector'} id "${options.id}" must start with a letter and be url-safe`
    )
  }
  const name = options.name?.trim() || titleCase(options.id)
  const description = options.description?.trim() || `${name} ${kind} for Vorn`
  const inRepo = options.repoConventions ?? false
  const module = kind === 'extension' ? 'extension' : 'connector'

  return [
    { path: 'package.json', contents: packageJson(options.id, description, inRepo, kind) },
    kind === 'extension'
      ? { path: 'src/extension.ts', contents: extensionSource(options.id, name, description) }
      : { path: 'src/connector.ts', contents: connectorSource(options.id, name, description) },
    { path: 'src/entry.ts', contents: entrySource(module) },
    { path: 'src/index.ts', contents: indexSource(module) },
    kind === 'extension'
      ? { path: 'src/extension.test.ts', contents: extensionTestSource(name) }
      : { path: 'src/connector.test.ts', contents: testSource(name) },
    { path: 'src/entry.test.ts', contents: entryTestSource() },
    // The page a pane is drawn from, carried into the pack as it stands here.
    ...(kind === 'extension'
      ? [{ path: 'web/report/index.html', contents: extensionPage(name) }]
      : []),
    {
      path: 'README.md',
      contents:
        kind === 'extension'
          ? extensionReadme(options.id, name, description)
          : readme(options.id, name, description)
    },
    // Everywhere: the generated source imports its package.json, which needs resolveJsonModule to compile.
    { path: 'tsconfig.json', contents: tsconfig(inRepo) },
    ...(inRepo
      ? [
          { path: 'CHANGELOG.md', contents: changelog() },
          { path: 'tsup.config.ts', contents: tsupConfig() },
          { path: 'vitest.config.ts', contents: vitestConfig() }
        ]
      : [])
  ]
}
