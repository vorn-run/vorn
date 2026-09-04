import { afterEach, describe, expect, it } from 'vitest'
import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { extract, list } from 'tar'
import {
  bundleDependencyFindings,
  defineConnector,
  lifecycleScriptFindings,
  packConnector,
  packFileName,
  readNearestPackageJson,
  MAX_PACK_BYTES
} from '../packages/connector-sdk/src/index'
import type { BundleOutput } from '../packages/connector-sdk/src/pack'
import type { CheckFinding } from '../packages/connector-sdk/src/check'
import { runCli } from '../packages/connector-sdk/src/cli'
import type { Connector } from '../packages/connector-sdk/src/types'

const connector: Connector = defineConnector({
  id: 'acme',
  name: 'Acme',
  version: '1.2.3',
  description: 'Acme tickets',
  triggers: [
    {
      type: 'newTicket',
      label: 'New ticket',
      description: 'Tickets opened since the last poll',
      poll: () => ({ items: [{ externalId: '1', title: 'Ticket 1' }] })
    }
  ]
})

const temps: string[] = []

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-pack-test-'))
  temps.push(dir)
  return dir
}

afterEach(() => {
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

const cleanBundle = async (): Promise<BundleOutput> => ({
  code: 'import { readFile } from "node:fs/promises"\nexport default readFile\n',
  external: ['node:fs/promises', 'path']
})

/** Stands in for starting the staged pack, where what is under test is the archive rather than the launch. */
const starts = async (): Promise<CheckFinding[]> => []

describe('pack gates', () => {
  it('passes a package with no install-time scripts', () => {
    expect(lifecycleScriptFindings(undefined)).toEqual([])
    expect(lifecycleScriptFindings({ scripts: { build: 'tsup', test: 'vitest' } })).toEqual([])
  })

  it('rejects install-time scripts by name', () => {
    const findings = lifecycleScriptFindings({
      scripts: { postinstall: 'node evil.js', prepare: 'husky', build: 'tsup' }
    })
    expect(findings).toHaveLength(1)
    expect(findings[0].code).toBe('lifecycle-scripts')
    expect(findings[0].message).toContain('postinstall, prepare')
    expect(findings[0].message).not.toContain('build')
  })

  it('accepts a bundle that only reaches for node builtins', async () => {
    expect(bundleDependencyFindings((await cleanBundle()).external)).toEqual([])
    expect(bundleDependencyFindings(['./chunk.js', '/abs/thing.js'])).toEqual([])
  })

  it('reports every bare specifier the bundler left behind', () => {
    const findings = bundleDependencyFindings(['zod', 'undici', '@scope/thing', 'node:fs'])
    expect(findings).toHaveLength(1)
    expect(findings[0].code).toBe('runtime-dependencies')
    expect(findings[0].message).toContain('@scope/thing, undici, zod')
  })

  it('finds the nearest package.json and gives up at the root', () => {
    const dir = tempDir()
    mkdirSync(join(dir, 'src', 'deep'), { recursive: true })
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: 'held' }))
    expect(readNearestPackageJson(join(dir, 'src', 'deep'))?.name).toBe('held')
    expect(readNearestPackageJson(tempDir())).toBeUndefined()
  })
})

describe('packConnector', () => {
  it('writes a gzipped tar holding the manifest and one bundled entry', async () => {
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './connector.js',
      resolveDir: tempDir(),
      outDir: out,
      bundle: cleanBundle,
      launch: starts
    })

    expect(result.file).toBe(join(out, 'acme-1.2.3.vorn.tgz'))
    expect(packFileName(connector)).toBe('acme-1.2.3.vorn.tgz')
    expect(result.bytes).toBeGreaterThan(0)
    expect(result.findings.some((item) => item.level === 'error')).toBe(false)

    const entries: string[] = []
    await list({ file: result.file as string, onReadEntry: (entry) => entries.push(entry.path) })
    expect(entries.sort()).toEqual(['index.js', 'manifest.json'])

    const unpacked = tempDir()
    await extract({ file: result.file as string, cwd: unpacked })
    const manifest = JSON.parse(readFileSync(join(unpacked, 'manifest.json'), 'utf8'))
    expect(manifest.id).toBe('acme')
    expect(manifest.version).toBe('1.2.3')
    expect(manifest.triggers[0].setup.filters.pollTool).toBe('poll_newTicket')
    expect(readFileSync(join(unpacked, 'index.js'), 'utf8')).toContain('node:fs/promises')
  })

  it('bundles the connector through a generated stdio entry', async () => {
    let seen = ''
    await packConnector(connector, {
      entry: 'acme-connector',
      outDir: tempDir(),
      resolveDir: tempDir(),
      bundle: async (request) => {
        seen = request.contents
        return cleanBundle()
      },
      launch: starts
    })
    expect(seen).toContain('serveConnector')
    expect(seen).toContain('"acme-connector"')
    expect(seen).toContain('"@vornrun/connector-sdk"')
  })

  it('packs nothing when the source package would run code at install time', async () => {
    const source = tempDir()
    writeFileSync(
      join(source, 'package.json'),
      JSON.stringify({ name: 'acme', scripts: { postinstall: 'node steal.js' } })
    )
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: source,
      outDir: out,
      bundle: cleanBundle
    })
    expect(result.file).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('lifecycle-scripts')
  })

  it('packs nothing when a dependency stayed outside the bundle', async () => {
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: tempDir(),
      bundle: async () => ({ code: 'import fetch from "undici"\n', external: ['undici'] })
    })
    expect(result.file).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('runtime-dependencies')
  })

  it('packs nothing when the bundle reads a file that packing leaves behind', async () => {
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: tempDir(),
      bundle: async () => ({
        code: 'createRequire(import.meta.url)("../package.json")\n',
        external: []
      })
    })
    expect(result.file).toBeUndefined()
    const deps = result.findings.find((item) => item.code === 'runtime-dependencies')
    expect(deps?.message).toContain('../package.json')
  })

  it('packs nothing when the staged bundle does not start', async () => {
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: out,
      // Loads, says a word, and exits: what a pack that reads a file beside it does on the host.
      bundle: async () => ({
        code: 'console.log("starting")\nthrow new Error("Cannot find module \'../package.json\'")\n',
        external: []
      })
    })
    expect(result.file).toBeUndefined()
    const launch = result.findings.find((item) => item.code === 'pack-launch')
    expect(launch?.message).toContain("Cannot find module '../package.json'")
    expect(() => readFileSync(join(out, 'acme-1.2.3.vorn.tgz'))).toThrow()
  }, 30_000)

  it('refuses a pack larger than the size ceiling and leaves no file behind', async () => {
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: out,
      maxBytes: 64,
      bundle: async () => ({
        code: `const filler = ${JSON.stringify('x'.repeat(4096))}\n`,
        external: []
      }),
      launch: starts
    })
    expect(result.file).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('pack-too-large')
    expect(() => readFileSync(join(out, 'acme-1.2.3.vorn.tgz'))).toThrow()
    expect(MAX_PACK_BYTES).toBe(8 * 1024 * 1024)
  })

  it('inlines every dependency when it really bundles', async () => {
    const source = tempDir()
    writeFileSync(join(source, 'package.json'), JSON.stringify({ name: 'real', type: 'module' }))
    writeFileSync(
      join(source, 'connector.js'),
      [
        `import { defineConnector } from ${JSON.stringify(resolve('packages/connector-sdk/src/index.ts'))}`,
        'export default defineConnector({',
        "  id: 'real',",
        "  name: 'Real',",
        "  version: '0.1.0',",
        "  description: 'A connector packed for real',",
        '  triggers: [',
        "    { type: 'thing', label: 'Thing', description: 'Things', poll: () => ({ items: [] }) }",
        '  ]',
        '})',
        ''
      ].join('\n')
    )
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './connector.js',
      resolveDir: source,
      outDir: out,
      sdkModule: resolve('packages/connector-sdk/src/index.ts')
    })

    expect(result.findings.filter((item) => item.level === 'error')).toEqual([])
    expect(result.file).toBe(join(out, 'acme-1.2.3.vorn.tgz'))
    const unpacked = tempDir()
    await extract({ file: result.file as string, cwd: unpacked })
    expect(readFileSync(join(unpacked, 'index.js'), 'utf8')).toContain('StdioServerTransport')
  }, 60_000)
})

describe('vorn-connector pack command', () => {
  const capture = (): { lines: string[]; write: (line: string) => void } => {
    const lines: string[] = []
    return { lines, write: (line) => lines.push(line) }
  }
  const load = async (): Promise<unknown> => ({ default: connector })

  it('writes the pack into the requested directory', async () => {
    const out = tempDir()
    const written = capture()
    const code = await runCli(['pack', './connector.js', '--out', out], {
      load,
      write: written.write,
      cwd: tempDir(),
      bundle: cleanBundle,
      launch: starts
    })
    expect(code).toBe(0)
    expect(written.lines.join('\n')).toContain(
      `Packed acme 1.2.3 to ${join(out, 'acme-1.2.3.vorn.tgz')}`
    )
    expect(readFileSync(join(out, 'acme-1.2.3.vorn.tgz')).byteLength).toBeGreaterThan(0)
  })

  it('fails with the findings when a gate rejects the connector', async () => {
    const out = capture()
    const code = await runCli(['pack', './connector.js', '--out', tempDir()], {
      load,
      write: out.write,
      cwd: tempDir(),
      bundle: async () => ({ code: 'import x from "left-pad"\n', external: ['left-pad'] })
    })
    expect(code).toBe(1)
    expect(out.lines.join('\n')).toContain('runtime-dependencies')
    expect(out.lines.join('\n')).toContain('nothing was packed')
  })

  it('lists pack in the usage text', async () => {
    const out = capture()
    await runCli(['help'], { load, write: out.write })
    expect(out.lines.join('\n')).toContain('pack <module>')
  })
})
