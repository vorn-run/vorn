import { describe, expect, it } from 'vitest'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFile } from 'node:child_process'
import {
  bundledRequireFindings,
  checkConnector,
  defineConnector,
  esbuildBundle,
  runConformance
} from '../packages/connector-sdk/src/index'
import type {
  ActionDefinition,
  ConnectorAuth,
  ConnectorConfigField,
  ConnectorDefinition
} from '../packages/connector-sdk/src/types'

const ping: ActionDefinition = {
  type: 'ping',
  label: 'Ping',
  description: 'Ask whether the service is up',
  idempotent: true,
  outputs: [{ key: 'ok', type: 'boolean' }],
  run: () => ({ ok: true })
}

/** A connector that is clean apart from whatever the case under test changes. */
const connector = (over: Partial<ConnectorDefinition> = {}) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    description: 'Talks to Acme',
    actions: [ping],
    ...over
  })

const codes = async (over: Partial<ConnectorDefinition> = {}) =>
  (await checkConnector(connector(over))).map((item) => item.code)

const cli = (probe: ConnectorAuth['probe']): ConnectorAuth => ({ rung: 'cli', probe })

describe('what a check says about signing in', () => {
  it('asks a connector that says nothing to say something', async () => {
    expect(await codes()).toContain('auth-undeclared')
  })

  it('passes a rung the host will keep whole', async () => {
    const auth = cli({ command: 'glab', args: ['auth', 'status'] })
    expect(await codes({ auth })).not.toContain('auth-probe-missing')
    expect(await codes({ auth })).not.toContain('auth-undeclared')
  })

  it('refuses a probe the host would drop, which is a rung promising a sign-in it cannot ask for', async () => {
    // Passes defineConnector — a non-empty command — and is dropped at the host.
    const found = await codes({ auth: cli({ command: '/usr/local/bin/glab' }) })
    expect(found).toContain('auth-probe-missing')
  })

  it('refuses probe arguments the host cannot pass on', async () => {
    const auth = cli({ command: 'glab', args: [7 as unknown as string] })
    expect(await codes({ auth })).toContain('auth-probe-missing')
  })

  it('asks nothing of a key rung, which has no probe to run', async () => {
    const config: ConnectorConfigField[] = [{ key: 'apiKey', label: 'API key', secret: true }]
    const auth: ConnectorAuth = { rung: 'key', keys: ['apiKey'] }
    expect(await codes({ auth, config })).not.toContain('auth-probe-missing')
  })
})

describe('what a check says about credentials', () => {
  it('refuses a named key that Vorn would store in the clear', async () => {
    const config: ConnectorConfigField[] = [{ key: 'apiKey', label: 'API key' }]
    const auth: ConnectorAuth = { rung: 'key', keys: ['apiKey'] }
    const findings = await checkConnector(connector({ auth, config }))
    const secret = findings.find((item) => item.code === 'secret-not-marked')
    expect(secret?.level).toBe('error')
  })

  it('warns about a field that reads like a credential even when nothing names it', async () => {
    const config: ConnectorConfigField[] = [{ key: 'webhookSecret', label: 'Webhook secret' }]
    const findings = await checkConnector(connector({ config }))
    const secret = findings.find((item) => item.code === 'secret-not-marked')
    expect(secret?.level).toBe('warn')
  })

  it('says nothing about a credential that is already marked', async () => {
    const config: ConnectorConfigField[] = [{ key: 'apiToken', label: 'Token', secret: true }]
    expect(await codes({ config })).not.toContain('secret-not-marked')
  })

  it('says nothing about an ordinary setting', async () => {
    const config: ConnectorConfigField[] = [{ key: 'project', label: 'Project' }]
    expect(await codes({ config })).not.toContain('secret-not-marked')
  })
})

describe('what a check says about an action', () => {
  it('asks an action with no declared outputs to name them', async () => {
    const bare: ActionDefinition = { ...ping, outputs: undefined }
    expect(await codes({ actions: [bare] })).toContain('action-no-outputs')
  })

  it('refuses an input type Vorn cannot draw', async () => {
    const odd: ActionDefinition = {
      ...ping,
      inputs: [{ key: 'when', label: 'When', type: 'date' as never }]
    }
    expect(await codes({ actions: [odd] })).toContain('input-type-unsupported')
  })

  it('refuses a select that offers no choices at all', async () => {
    const empty: ActionDefinition = {
      ...ping,
      inputs: [{ key: 'level', label: 'Level', type: 'select' }]
    }
    expect(await codes({ actions: [empty] })).toContain('input-type-unsupported')
  })

  it('accepts a select that knows its choices, and one that fetches them', async () => {
    const fixed: ActionDefinition = {
      ...ping,
      inputs: [{ key: 'level', label: 'Level', type: 'select', options: [{ value: 'high' }] }]
    }
    const loaded: ActionDefinition = {
      ...ping,
      inputs: [{ key: 'channel', label: 'Channel', type: 'select', loadOptions: 'channels' }]
    }
    expect(await codes({ actions: [fixed] })).not.toContain('input-type-unsupported')
    expect(await codes({ actions: [loaded], options: { channels: () => [] } })).not.toContain(
      'input-type-unsupported'
    )
  })
})

describe('what a check says about the package a connector ships as', () => {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-check-pkg-'))
  const writePackage = (pkg: Record<string, unknown>) =>
    writeFileSync(join(dir, 'package.json'), JSON.stringify(pkg))

  it('says nothing about a package it was never pointed at', async () => {
    expect(await codes()).not.toContain('keywords-missing')
  })

  it('refuses a package whose install would run code', async () => {
    writePackage({
      name: 'acme',
      scripts: { postinstall: 'node evil.js' },
      vorn: { keywords: ['acme'] }
    })
    const findings = await checkConnector(connector(), { packageDir: dir })
    const lifecycle = findings.find((item) => item.code === 'lifecycle-scripts')
    expect(lifecycle?.level).toBe('error')
  })

  it('asks for keywords, because a catalog of fifty is searched rather than read', async () => {
    writePackage({ name: 'acme' })
    const findings = await checkConnector(connector(), { packageDir: dir })
    expect(findings.map((item) => item.code)).toContain('keywords-missing')
  })

  it('refuses what would still need a registry at launch', async () => {
    writePackage({ name: 'acme', vorn: { keywords: ['acme'] } })
    const findings = await checkConnector(connector(), {
      packageDir: dir,
      entry: './index.js',
      bundle: async () => ({ code: '', external: ['left-pad', 'node:fs', './local.js'] })
    })
    const deps = findings.find((item) => item.code === 'runtime-dependencies')
    expect(deps?.level).toBe('error')
    // Builtins and relative files are not dependencies a pack has to carry.
    expect(deps?.message).toContain('left-pad')
    expect(deps?.message).not.toContain('node:fs')
  })

  it('judges the entry package, not the directory the command was run from', async () => {
    // A monorepo: the root is clean, the connector's own package is not.
    const root = mkdtempSync(join(tmpdir(), 'vorn-check-root-'))
    writeFileSync(join(root, 'package.json'), JSON.stringify({ vorn: { keywords: ['root'] } }))
    const own = join(root, 'packages', 'slack')
    mkdirSync(join(own, 'dist'), { recursive: true })
    writeFileSync(
      join(own, 'package.json'),
      JSON.stringify({ name: 'slack', scripts: { postinstall: 'node setup.js' } })
    )

    const findings = await checkConnector(connector(), {
      packageDir: root,
      entry: './packages/slack/dist/index.js'
    })

    const codes = findings.map((item) => item.code)
    expect(codes).toContain('lifecycle-scripts')
    // Slack's package names no keywords; the root's do not stand in for them.
    expect(codes).toContain('keywords-missing')
  })

  it('judges the working directory when the entry is a bare specifier', async () => {
    const root = mkdtempSync(join(tmpdir(), 'vorn-check-bare-'))
    writeFileSync(join(root, 'package.json'), JSON.stringify({ vorn: { keywords: ['root'] } }))

    const findings = await checkConnector(connector(), {
      packageDir: root,
      entry: '@acme/connector-slack'
    })
    expect(findings.map((item) => item.code)).not.toContain('keywords-missing')
  })

  it('passes a package that bundles everything it needs', async () => {
    writePackage({ name: 'acme', vorn: { keywords: ['acme'] } })
    const findings = await checkConnector(connector(), {
      packageDir: dir,
      entry: './index.js',
      bundle: async () => ({ code: '', external: [] })
    })
    expect(findings.map((item) => item.code)).not.toContain('runtime-dependencies')
  })
})

/** A bundle that serves the MCP handshake Vorn opens with, then waits like a served connector. */
const SERVES = [
  "let buffered = ''",
  "process.stdin.on('data', (chunk) => {",
  '  buffered += chunk',
  '  for (;;) {',
  "    const end = buffered.indexOf('\\n')",
  '    if (end < 0) return',
  '    const line = buffered.slice(0, end)',
  '    buffered = buffered.slice(end + 1)',
  '    if (!line.trim()) continue',
  '    const message = JSON.parse(line)',
  "    if (message.method !== 'initialize') continue",
  '    const result = {',
  '      protocolVersion: message.params.protocolVersion,',
  '      capabilities: {},',
  "      serverInfo: { name: 'stub', version: '1' }",
  '    }',
  "    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }) + '\\n')",
  '  }',
  '})',
  ''
].join('\n')

/** What every connector but one shipped: a require the bundler left for the runtime. */
const READS_ITS_PACKAGE =
  'import { createRequire } from "node:module"\ncreateRequire(import.meta.url)("../package.json")\n' +
  SERVES

/** A bundle that says something cheerful on its way out, which is not an answer. */
const DIES_ON_LOAD = 'console.log("booting")\nthrow new Error("boom")\n'

describe('what a check says about starting the pack it would ship', () => {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-check-launch-'))
  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({ name: 'acme', vorn: { keywords: ['acme'] } })
  )

  const check = (code: string, over: Record<string, unknown> = {}) =>
    checkConnector(connector(), {
      mock: true,
      packageDir: dir,
      entry: './index.js',
      bundle: async () => ({ code, external: [] }),
      ...over
    })

  it('starts a bundle that carries everything it needs', async () => {
    const findings = await check(SERVES)
    expect(findings.map((item) => item.code)).not.toContain('pack-launch')
  })

  it('refuses a bundle that starts, says something, and never answers', async () => {
    const findings = await check(DIES_ON_LOAD)
    const launch = findings.find((item) => item.code === 'pack-launch')
    expect(launch?.level).toBe('error')
    // The line naming the failure, not the banner the connector logged first.
    expect(launch?.message).toContain('boom')
  })

  it('refuses a bundle that starts and then serves nothing', async () => {
    expect(await check('')).toContainEqual(
      expect.objectContaining({ code: 'pack-launch', level: 'error' })
    )
  })

  it('advises about a file only the source tree has, and refuses it for not starting', async () => {
    const findings = await check(READS_ITS_PACKAGE)
    // The scan names the suspect; starting the pack is what condemns it.
    expect(findings).toContainEqual(
      expect.objectContaining({ code: 'runtime-dependencies', level: 'warn' })
    )
    expect(findings).toContainEqual(
      expect.objectContaining({ code: 'pack-launch', level: 'error' })
    )
  })

  it('starts a bundle whose CommonJS dependency asks for a builtin', async () => {
    // What every connector wrapping a published CJS client hits: esbuild's shim throws unless a real require is in scope.
    const source = mkdtempSync(join(tmpdir(), 'vorn-check-cjs-'))
    writeFileSync(
      join(source, 'legacy.cjs'),
      'const url = require("url")\nmodule.exports = url.URL\n'
    )
    const built = await esbuildBundle({
      contents: 'import URL from "./legacy.cjs"\nif (!URL) throw new Error("no URL")\n',
      resolveDir: source
    })
    writeFileSync(join(source, 'bundle.mjs'), built.code)

    const ran = await new Promise<number | null>((resolve) => {
      execFile(process.execPath, [join(source, 'bundle.mjs')], (error) =>
        resolve(error ? ((error as { code?: number }).code ?? 1) : 0)
      )
    })

    expect(ran).toBe(0)
  })

  it('starts nothing without a mock run, which is where the packaging gates live', async () => {
    const findings = await check(DIES_ON_LOAD, { mock: false })
    expect(findings.map((item) => item.code)).not.toContain('pack-launch')
  })

  it('vouches for the launch only when it happened', async () => {
    const served = await runConformance(connector(), {
      mock: true,
      packageDir: dir,
      entry: './index.js',
      bundle: async () => ({ code: SERVES, external: [] })
    })
    expect(served.passed).toContain('launch')
    expect((await runConformance(connector(), { mock: true })).passed).not.toContain('launch')
  })

  it('names the file a bundle asks for after packing, wherever the require came from', () => {
    const named = (code: string) => bundledRequireFindings(code)[0]?.message ?? ''
    expect(named('createRequire(import.meta.url)("../package.json")')).toContain('../package.json')
    expect(named('require("./schema.json")')).toContain('./schema.json')
    expect(named('__require("../data/rows.json")')).toContain('../data/rows.json')
    expect(named('require.resolve("../rows.json")')).toContain('../rows.json')
    expect(named('await import("./late.js")')).toContain('./late.js')
    // The helper esbuild emits for bundled CommonJS is not a file left behind.
    expect(bundledRequireFindings('var __require = createRequire(import.meta.url)')).toEqual([])
    expect(bundledRequireFindings('const x = require("node:fs")')).toEqual([])
    expect(bundledRequireFindings('import { join } from "./path.js"')).toEqual([])
  })

  it('advises rather than refuses, since starting the pack is what settles it', () => {
    expect(bundledRequireFindings('require("./schema.json")')[0]?.level).toBe('warn')
  })

  it('reads a bundle as code, so a dependency that only writes the words is left alone', () => {
    // What a minified dependency carries: a JSDoc type, an example, a message.
    expect(bundledRequireFindings('/** @type {import("./get")} */\nvar x = 1')).toEqual([])
    expect(bundledRequireFindings('// require("./get") is what the old build did\n')).toEqual([])
    expect(bundledRequireFindings('var hint = "call require(\'./get\') yourself"')).toEqual([])
    expect(bundledRequireFindings('var hint = `require("./get")`')).toEqual([])
    // A quote inside a regular expression does not open a string that hides the call after it.
    const afterRegex = 'var q = text.replace(/"/g, "")\nrequire("./late.js")'
    expect(bundledRequireFindings(afterRegex)[0]?.message).toContain('./late.js')
    // A property of that name is not the loader.
    expect(bundledRequireFindings('loader.require("./get")')).toEqual([])
  })
})
