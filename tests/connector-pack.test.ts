import { afterEach, describe, expect, it } from 'vitest'
import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
  mkdirSync,
  type Dirent
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative, resolve } from 'node:path'
import {
  bundleDependencyFindings,
  defineConnector,
  defineExtension,
  lifecycleScriptFindings,
  packConnector,
  readNearestPackageJson,
  MANIFEST_FILE
} from '../packages/connector-sdk/src/index'
import type { BundleOutput } from '../packages/connector-sdk/src/pack'
import { extrasOf } from '../packages/connector-sdk/src/pack'
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

/** Every file under `dir`, relative to it, in order. */
function files(dir: string): string[] {
  return (readdirSync(dir, { recursive: true, withFileTypes: true }) as Dirent[])
    .filter((e) => e.isFile())
    .map((e) => relative(dir, join(e.parentPath, e.name)))
    .sort()
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
  it('writes a folder holding the manifest and one bundled entry', async () => {
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './connector.js',
      resolveDir: tempDir(),
      outDir: out,
      bundle: cleanBundle,
      launch: starts,
      extras: { hosts: ['api.acme.example'], publisher: 'Acme' }
    })

    expect(result.dir).toBe(join(out, 'acme'))
    expect(result.bytes).toBeGreaterThan(0)
    expect(result.findings.some((item) => item.level === 'error')).toBe(false)
    expect(files(result.dir as string)).toEqual(['index.js', MANIFEST_FILE])

    const manifest = readFileSync(join(result.dir as string, MANIFEST_FILE), 'utf8')
    expect(manifest).toContain(
      'id = "acme"\nname = "Acme"\nversion = "1.2.3"\napi = "vorn:extension@1.0"'
    )
    expect(manifest).toContain('publisher = "Acme"')
    expect(manifest).toContain('[service]\ncommand = ["node", "index.js"]')
    expect(manifest).toContain('[checks]\ncommand = ["node", "index.js", "--check"]')
    expect(manifest).toContain('[declares]\nnetwork = ["api.acme.example"]')
    expect(manifest).toContain('[[connector.triggers]]\ntype = "newTicket"\nlabel = "New ticket"')
    expect(readFileSync(join(result.dir as string, 'index.js'), 'utf8')).toContain(
      'node:fs/promises'
    )
  })

  it('carries the receipt of its checks and keeps what else the folder holds', async () => {
    const source = tempDir()
    writeFileSync(join(source, 'package.json'), JSON.stringify({ name: 'acme' }))
    writeFileSync(
      join(source, 'verified.json'),
      JSON.stringify({ schema: 1, version: '1.2.3', checks: ['manifest'] })
    )
    const out = tempDir()
    mkdirSync(join(out, 'acme'), { recursive: true })
    writeFileSync(join(out, 'acme', 'README.md'), 'kept')
    const result = await packConnector(connector, {
      entry: './connector.js',
      resolveDir: source,
      outDir: out,
      bundle: cleanBundle,
      launch: starts
    })
    expect(files(result.dir as string)).toEqual([
      'README.md',
      'index.js',
      'verified.json',
      MANIFEST_FILE
    ])
    expect(readFileSync(join(out, 'acme', 'README.md'), 'utf8')).toBe('kept')
  })

  it('reads the hosts and the publisher from vorn in package.json', () => {
    expect(
      extrasOf({
        author: 'Ada Lovelace <ada@example.com>',
        vorn: { hosts: ['api.example.com', 3], hostsFrom: ['FEEDS'] }
      })
    ).toEqual({ hosts: ['api.example.com'], hostsFrom: ['FEEDS'], publisher: 'Ada Lovelace' })
    expect(extrasOf(undefined)).toEqual({
      hosts: undefined,
      hostsFrom: undefined,
      publisher: undefined
    })
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
    expect(seen).toContain('--check')
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
    expect(result.dir).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('lifecycle-scripts')
  })

  it('packs nothing when a dependency stayed outside the bundle', async () => {
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: tempDir(),
      bundle: async () => ({ code: 'import fetch from "undici"\n', external: ['undici'] })
    })
    expect(result.dir).toBeUndefined()
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
    expect(result.dir).toBeUndefined()
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
    expect(result.dir).toBeUndefined()
    const launch = result.findings.find((item) => item.code === 'pack-launch')
    expect(launch?.message).toContain("Cannot find module '../package.json'")
    expect(existsSync(join(out, 'acme'))).toBe(false)
  }, 30_000)

  it('refuses a package larger than the size ceiling and leaves no folder behind', async () => {
    const out = tempDir()
    const result = await packConnector(connector, {
      entry: './index.js',
      resolveDir: tempDir(),
      outDir: out,
      maxUnpackedBytes: 64,
      bundle: async () => ({
        code: `const filler = ${JSON.stringify('x'.repeat(4096))}\n`,
        external: []
      }),
      launch: starts
    })
    expect(result.dir).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('pack-too-large')
    expect(existsSync(join(out, 'acme'))).toBe(false)
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
    expect(result.dir).toBe(join(out, 'acme'))
    expect(readFileSync(join(out, 'acme', 'index.js'), 'utf8')).toContain('vorn/hello')
  }, 60_000)
})

describe('packing an extension', () => {
  const extension = defineExtension({
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    description: 'Reads the session',
    permissions: [],
    panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }]
  })

  /** A package carrying the page, and the stylesheet beside it that the page asks for. */
  function packageWithPage(): string {
    const dir = tempDir()
    mkdirSync(join(dir, 'web', 'report'), { recursive: true })
    writeFileSync(join(dir, 'web', 'report', 'index.html'), '<!doctype html><link href=x.css>')
    writeFileSync(join(dir, 'web', 'report', 'x.css'), 'body { margin: 0 }')
    return dir
  }

  it('carries the page a pane names, and everything beside it', async () => {
    const out = tempDir()
    const result = await packConnector(extension, {
      entry: './extension.js',
      resolveDir: packageWithPage(),
      outDir: out,
      bundle: cleanBundle,
      launch: starts
    })

    expect(result.findings.some((item) => item.level === 'error')).toBe(false)
    expect(files(result.dir as string)).toEqual([
      'index.js',
      MANIFEST_FILE,
      'web/report/index.html',
      'web/report/x.css'
    ])
    const manifest = readFileSync(join(result.dir as string, MANIFEST_FILE), 'utf8')
    // The manifest's path resolves inside the folder, because the directory was carried as it stood.
    expect(manifest).toContain(
      '[[contributes.panes]]\nid = "report"\ntitle = "Report"\nweb = "web/report/index.html"'
    )
    expect(readFileSync(join(result.dir as string, 'web/report/index.html'), 'utf8')).toContain(
      'doctype'
    )
  })

  it('finds the page beside the package when the entry is the built bundle', async () => {
    // What the CLI really passes: `./dist/index.js`, whose directory holds no `web/`.
    const dir = packageWithPage()
    mkdirSync(join(dir, 'dist'), { recursive: true })
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: 'review' }))

    const result = await packConnector(extension, {
      entry: './dist/index.js',
      resolveDir: dir,
      outDir: tempDir(),
      bundle: cleanBundle,
      launch: starts
    })

    expect(result.findings.some((item) => item.level === 'error')).toBe(false)
    expect(files(result.dir as string)).toContain('web/report/index.html')
  })

  it('packs nothing when a pane names a page the package does not carry', async () => {
    const result = await packConnector(extension, {
      entry: './extension.js',
      resolveDir: tempDir(),
      outDir: tempDir(),
      bundle: cleanBundle,
      launch: starts
    })

    expect(result.dir).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('web-entry-missing')
  })

  it('carries only what a pane names, not everything left under web/', async () => {
    const dir = packageWithPage()
    // An unrelated build, a scratch page, a stray env file: in the tree, not in the pack.
    mkdirSync(join(dir, 'web', 'scratch'), { recursive: true })
    writeFileSync(join(dir, 'web', 'scratch', 'index.html'), '<!doctype html>')
    writeFileSync(join(dir, 'web', 'notes.txt'), 'todo')

    const result = await packConnector(extension, {
      entry: './extension.js',
      resolveDir: dir,
      outDir: tempDir(),
      bundle: cleanBundle,
      launch: starts
    })

    const entries = files(result.dir as string)
    expect(entries).not.toContain('web/scratch/index.html')
    expect(entries).not.toContain('web/notes.txt')
    expect(entries).toContain('web/report/index.html')
  })

  it('refuses a package past what Vorn keeps on disk', async () => {
    const dir = packageWithPage()
    // Pages compress well, so the archive can sit under its own ceiling while
    // the tree it unpacks to does not.
    writeFileSync(join(dir, 'web', 'report', 'big.txt'), 'a'.repeat(4 * 1024 * 1024))

    const result = await packConnector(extension, {
      entry: './extension.js',
      resolveDir: dir,
      outDir: tempDir(),
      bundle: cleanBundle,
      launch: starts,
      maxUnpackedBytes: 1024 * 1024
    })

    expect(result.dir).toBeUndefined()
    expect(result.findings.map((item) => item.code)).toContain('pack-too-large')
    expect(result.findings.at(-1)?.message).toContain('Vorn keeps at most')
  })

  it('carries no web directory for a connector, which has no pages', async () => {
    const result = await packConnector(connector, {
      entry: './connector.js',
      resolveDir: tempDir(),
      outDir: tempDir(),
      bundle: cleanBundle,
      launch: starts
    })

    expect(files(result.dir as string)).toEqual(['index.js', MANIFEST_FILE])
  })
})

describe('vorn-connector pack command', () => {
  const capture = (): { lines: string[]; write: (line: string) => void } => {
    const lines: string[] = []
    return { lines, write: (line) => lines.push(line) }
  }
  const load = async (): Promise<unknown> => ({ default: connector })

  it('writes the folder into the requested directory', async () => {
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
    expect(written.lines.join('\n')).toContain(`Packed acme 1.2.3 to ${join(out, 'acme')}`)
    expect(written.lines.join('\n')).toContain('vorn ext install <git url>')
    expect(readFileSync(join(out, 'acme', MANIFEST_FILE)).byteLength).toBeGreaterThan(0)
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
