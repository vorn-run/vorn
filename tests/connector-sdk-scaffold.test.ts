import { describe, expect, it } from 'vitest'
import { scaffoldFiles, titleCase } from '../packages/connector-sdk/src/index'
import { runCli } from '../packages/connector-sdk/src/cli'

const fileMap = (id = 'acme-tickets') =>
  new Map(scaffoldFiles({ id }).map((file) => [file.path, file.contents]))

describe('the files a new connector starts as', () => {
  it('writes everything build, check and pack expect to find', () => {
    expect([...fileMap().keys()]).toEqual([
      'package.json',
      'src/connector.ts',
      'src/entry.ts',
      'src/index.ts',
      'src/connector.test.ts',
      'src/entry.test.ts',
      'README.md'
    ])
  })

  it('names the connector after its id when nobody said otherwise', () => {
    expect(titleCase('acme-tickets')).toBe('Acme Tickets')
    expect(fileMap().get('src/connector.ts')).toContain('name: "Acme Tickets"')
    expect(scaffoldFiles({ id: 'acme', name: 'Acme Corp' })[1].contents).toContain(
      'name: "Acme Corp"'
    )
  })

  it('gives the package the scripts the factory runs, and the block the catalog reads', () => {
    const pkg = JSON.parse(fileMap().get('package.json') as string) as {
      name: string
      scripts: Record<string, string>
      dependencies: Record<string, string>
      vorn: { category: string; keywords: string[] }
    }

    expect(pkg.name).toBe('vorn-connector-acme-tickets')
    expect(pkg.scripts.check).toBe('vorn-connector check src/index.ts')
    expect(pkg.scripts.pack).toBe('vorn-connector pack src/index.ts')
    // Names a prerelease because that is all there is: a bare `^0.7.0` matches
    // no prerelease, so a scaffold pinned to it would install nothing.
    expect(pkg.dependencies['@vornrun/connector-sdk']).toBe('^0.7.0-beta.9')
    expect(pkg.vorn.keywords).toContain('acme-tickets')
  })

  it('starts from the SDK it is being written against: declared, with an auth rung', () => {
    const source = fileMap().get('src/connector.ts') as string
    expect(source).toContain("version: '0.1.0'")
    const quoted = scaffoldFiles({ id: 'acme', name: "Bob's App", description: 'Two\nlines' })
    const connector = quoted.find((f) => f.path === 'src/connector.ts')?.contents ?? ''
    expect(connector).toContain('name: "Bob\'s App"')
    expect(connector).toContain('description: "Two\\nlines"')
    expect(source).toContain('auth: { rung:')
    expect(source).toContain('request: {')
    expect(source).toContain('postReceive:')
    expect(source).toContain('dedupe:')
    expect(source).toContain('builderHint:')
    // The generated connector uses the retrying fetch, not the global one.
    expect(source).toContain('context.fetch(')
  })

  it('starts with a test that needs no network', () => {
    const test = fileMap().get('src/connector.test.ts') as string
    expect(test).toContain('createConnectorHarness')
    expect(test).toContain('fetchImpl')
    expect(test).toContain('pollTwice')
  })

  it('refuses an id that could not be a connector id', () => {
    expect(() => scaffoldFiles({ id: '1nope' })).toThrow(/must start with a letter/)
    expect(() => scaffoldFiles({ id: 'a/b' })).toThrow(/url-safe/)
  })
})

describe('a scaffold shaped for the connectors repository', () => {
  const generated = new Map<string, Map<string, string>>()
  const repoMap = (id = 'acme-tickets') => {
    const cached = generated.get(id)
    if (cached) return cached
    const files = new Map(
      scaffoldFiles({ id, repoConventions: true }).map((file) => [file.path, file.contents])
    )
    generated.set(id, files)
    return files
  }

  it('carries the files a package in that repository has to have', () => {
    expect([...repoMap().keys()]).toEqual([
      'package.json',
      'src/connector.ts',
      'src/entry.ts',
      'src/index.ts',
      'src/connector.test.ts',
      'src/entry.test.ts',
      'README.md',
      'CHANGELOG.md',
      'tsconfig.json',
      'tsup.config.ts',
      'vitest.config.ts'
    ])
  })

  it('takes the scoped name and lists the changelog among what it publishes', () => {
    const pkg = JSON.parse(repoMap().get('package.json') as string) as {
      name: string
      version: string
      files: string[]
      repository: { directory: string }
      vorn: { auth?: string }
    }

    expect(pkg.name).toBe('@vornrun/connector-acme-tickets')
    expect(pkg.files).toEqual(['dist', 'README.md', 'CHANGELOG.md'])
    expect(pkg.repository.directory).toBe('packages/acme-tickets')
    // The catalog prints this line; the scaffold leaves a prompt rather than a guess.
    expect(pkg.vorn.auth).toBeTruthy()
  })

  it('answers to the one test configuration the repository keeps, not its own', () => {
    expect(repoMap().get('vitest.config.ts')).toContain('vitest.shared')
  })

  it('opens the changelog at the version the package claims', () => {
    const pkg = JSON.parse(repoMap().get('package.json') as string) as { version: string }
    expect(repoMap().get('CHANGELOG.md')).toContain(`## ${pkg.version}`)
  })

  it('builds the way the other packages build', () => {
    const tsup = repoMap().get('tsup.config.ts') as string
    expect(tsup).toContain("target: 'node22'")
    expect(tsup).toContain("banner: { js: '#!/usr/bin/env node' }")
    expect(tsup).toContain('dts: true')
    expect(repoMap().get('tsconfig.json')).toContain('"noEmit": true')
    expect(repoMap().get('tsconfig.json')).toContain('"allowImportingTsExtensions": true')
    expect(repoMap().get('tsconfig.json')).toContain('"vitest.config.ts"')
    const pkg = JSON.parse(repoMap().get('package.json') ?? '{}') as {
      devDependencies: Record<string, string>
    }
    expect(Object.keys(pkg.devDependencies)).toEqual(
      expect.arrayContaining(['@types/node', '@vitest/coverage-v8'])
    )
  })

  it('leaves a scaffold nobody asked to shape exactly as it was', () => {
    const plain = new Map(scaffoldFiles({ id: 'acme' }).map((f) => [f.path, f.contents]))
    expect([...plain.keys()]).not.toContain('CHANGELOG.md')
    expect(plain.get('package.json')).toContain('"name": "vorn-connector-acme"')
    expect(plain.get('package.json')).not.toContain('repository')
  })
})

describe('vorn-connector new', () => {
  const capture = () => {
    const lines: string[] = []
    return { lines, write: (line: string) => lines.push(line) }
  }
  const load = async (): Promise<unknown> => {
    throw new Error('new must not load a module')
  }

  it('writes the scaffold under the id, and says what it made', async () => {
    const written = new Map<string, string>()
    const out = capture()

    const code = await runCli(['new', 'acme', '--out', '/tmp/work'], {
      load,
      write: out.write,
      writeFile: async (path, contents) => {
        written.set(path, contents)
      }
    })

    expect(code).toBe(0)
    expect([...written.keys()]).toContain('/tmp/work/acme/src/connector.ts')
    expect([...written.keys()]).toContain('/tmp/work/acme/package.json')
    expect(out.lines.join('\n')).toContain('Created acme in /tmp/work/acme')
  })

  it('shapes the package for the connectors repository when asked, and only then', async () => {
    const written = new Map<string, string>()
    const writeFile = async (path: string, contents: string) => {
      written.set(path, contents)
    }
    await runCli(['new', 'acme', '--repo-conventions', '--out', '/tmp/work'], {
      load,
      write: capture().write,
      writeFile
    })

    expect([...written.keys()]).toContain('/tmp/work/acme/vitest.config.ts')
    expect(written.get('/tmp/work/acme/package.json')).toContain('"@vornrun/connector-acme"')

    written.clear()
    await runCli(['new', 'acme', '--out', '/tmp/work'], {
      load,
      write: capture().write,
      writeFile
    })
    expect([...written.keys()]).not.toContain('/tmp/work/acme/vitest.config.ts')
  })

  it('takes a display name, and reports an id it cannot use', async () => {
    const written = new Map<string, string>()
    const writeFile = async (path: string, contents: string) => {
      written.set(path, contents)
    }
    await runCli(['new', 'acme', '--name', 'Acme Corp', '--out', '/tmp/work'], {
      load,
      write: capture().write,
      writeFile
    })
    expect(written.get('/tmp/work/acme/src/connector.ts')).toContain('name: "Acme Corp"')

    await expect(
      runCli(['new', '1nope'], { load, write: capture().write, writeFile })
    ).rejects.toThrow(/must start with a letter/)
  })

  it('is listed in the usage text', async () => {
    const help = capture()
    await runCli(['help'], { load, write: help.write })
    expect(help.lines.join('\n')).toContain('new <id>')
    expect(help.lines.join('\n')).toContain('--repo-conventions')
  })
})
