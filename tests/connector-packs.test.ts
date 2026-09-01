import { afterEach, describe, expect, it } from 'vitest'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { create } from 'tar'
import {
  MAX_PACK_BYTES,
  describePack,
  inspectPack,
  installPack,
  installedLaunch,
  isSafeArchiveEntry,
  listInstalledPacks,
  removePack,
  rollbackPack,
  verifyPackDir
} from '../packages/server/src/connectors/packs'
import type { ConnectorInstallProgress } from '@vornrun/shared/types'

const temps: string[] = []

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-packs-test-'))
  temps.push(dir)
  return dir
}

afterEach(() => {
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

function manifestFor(id: string, version: string, name = 'Acme'): unknown {
  return {
    id,
    name,
    version,
    description: 'Acme tickets',
    triggers: [
      {
        type: 'newTicket',
        label: 'New ticket',
        setup: {
          filters: { pollTool: 'poll_newTicket' },
          env: [{ name: 'API_TOKEN', required: true, secret: true }]
        }
      }
    ],
    actions: [{ type: 'closeTicket', label: 'Close ticket' }]
  }
}

/** Build a `.vorn.tgz` on disk from a map of relative path to contents. */
async function buildArchive(files: Record<string, string>, prefix = ''): Promise<string> {
  const staging = tempDir()
  const base = prefix ? join(staging, prefix) : staging
  for (const [path, contents] of Object.entries(files)) {
    const full = join(base, path)
    mkdirSync(join(full, '..'), { recursive: true })
    writeFileSync(full, contents)
  }
  const file = join(tempDir(), 'pack.tgz')
  await create({ gzip: true, file, cwd: staging }, prefix ? [prefix] : Object.keys(files))
  return file
}

function goodFiles(version = '1.2.0', id = 'acme'): Record<string, string> {
  return {
    'manifest.json': JSON.stringify(manifestFor(id, version)),
    'index.js': 'process.stdin.resume()\n'
  }
}

describe('archive entry safety', () => {
  it('accepts ordinary files and directories', () => {
    expect(isSafeArchiveEntry('index.js', 'File')).toBe(true)
    expect(isSafeArchiveEntry('nested/deep/index.js', 'Directory')).toBe(true)
  })

  it('rejects traversal, absolute paths and links of either kind', () => {
    expect(isSafeArchiveEntry('../escape.js', 'File')).toBe(false)
    expect(isSafeArchiveEntry('nested/../../escape.js', 'File')).toBe(false)
    expect(isSafeArchiveEntry('/etc/passwd', 'File')).toBe(false)
    expect(isSafeArchiveEntry('C:/windows/system32', 'File')).toBe(false)
    expect(isSafeArchiveEntry('link.js', 'SymbolicLink')).toBe(false)
    expect(isSafeArchiveEntry('link.js', 'Link')).toBe(false)
  })
})

describe('verifyPackDir', () => {
  const dirWith = (files: Record<string, string>): string => {
    const dir = tempDir()
    for (const [path, contents] of Object.entries(files)) {
      const full = join(dir, path)
      mkdirSync(join(full, '..'), { recursive: true })
      writeFileSync(full, contents)
    }
    return dir
  }

  it('accepts a manifest and one entry', () => {
    const manifest = verifyPackDir(dirWith(goodFiles()))
    expect(manifest.id).toBe('acme')
    expect(manifest.version).toBe('1.2.0')
    expect(manifest.triggers[0].filters.pollTool).toBe('poll_newTicket')
    expect(manifest.env[0].name).toBe('API_TOKEN')
  })

  it('refuses a pack with no manifest', () => {
    expect(() => verifyPackDir(dirWith({ 'index.js': '' }))).toThrow(/no manifest.json/)
  })

  it('refuses a manifest that is not readable as one', () => {
    expect(() => verifyPackDir(dirWith({ 'manifest.json': 'not json', 'index.js': '' }))).toThrow(
      /no readable manifest.json/
    )
    expect(() =>
      verifyPackDir(dirWith({ 'manifest.json': '{"name":"No id"}', 'index.js': '' }))
    ).toThrow(/missing an id or a name/)
  })

  it('refuses a pack with no entry or with more than one', () => {
    expect(() =>
      verifyPackDir(dirWith({ 'manifest.json': JSON.stringify(manifestFor('acme', '1.0.0')) }))
    ).toThrow(/no entry to run/)
    expect(() => verifyPackDir(dirWith({ ...goodFiles(), 'other.js': '' }))).toThrow(
      /exactly one index.js/
    )
  })

  it('refuses a package that would need an install step', () => {
    expect(() =>
      verifyPackDir(
        dirWith({ ...goodFiles(), 'package.json': JSON.stringify({ dependencies: { zod: '^4' } }) })
      )
    ).toThrow(/declares dependencies/)
    expect(() =>
      verifyPackDir(
        dirWith({
          ...goodFiles(),
          'package.json': JSON.stringify({ scripts: { postinstall: 'node evil.js' } })
        })
      )
    ).toThrow(/declares scripts/)
  })

  it('ignores an empty dependency block and an unparseable package.json', () => {
    expect(() =>
      verifyPackDir(
        dirWith({
          ...goodFiles(),
          'package.json': JSON.stringify({ name: 'acme', dependencies: {}, scripts: {} }),
          'nested/package.json': 'not json'
        })
      )
    ).not.toThrow()
  })

  it('refuses a pack that unpacks to more than the ceiling', () => {
    expect(() =>
      verifyPackDir(dirWith({ ...goodFiles(), 'index.js': 'x'.repeat(33 * 1024 * 1024) }))
    ).toThrow(/unpacks to/)
  })
})

describe('inspectPack', () => {
  it('describes what a pack would install without keeping any of it', async () => {
    const root = tempDir()
    const file = await buildArchive(goodFiles())

    const result = await inspectPack({ kind: 'file', path: file }, { root })

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.preview.id).toBe('acme')
    expect(result.preview.version).toBe('1.2.0')
    expect(result.preview.triggers.map((t) => t.label)).toEqual(['New ticket'])
    expect(result.preview.actions.map((a) => a.label)).toEqual(['Close ticket'])
    expect(result.preview.env.map((e) => e.name)).toEqual(['API_TOKEN'])
    expect(result.preview.installedVersion).toBeUndefined()
    // Nothing was kept: no connector directory, and no staging left behind.
    expect(readdirSync(root)).toEqual([])
  })

  it('names the version an install would replace', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.2.0')) }, { root })

    const result = await inspectPack(
      { kind: 'file', path: await buildArchive(goodFiles('1.3.0')) },
      { root }
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.preview.installedVersion).toBe('1.2.0')
    expect(result.preview.version).toBe('1.3.0')
  })

  it('refuses a pack that declares dependencies, leaving the disk untouched', async () => {
    const root = tempDir()
    const file = await buildArchive({
      ...goodFiles(),
      'package.json': JSON.stringify({ dependencies: { 'left-pad': '1.0.0' } })
    })

    const result = await inspectPack({ kind: 'file', path: file }, { root })

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/dependencies/)
    expect(readdirSync(root)).toEqual([])
  })

  it('reports a source that cannot be read rather than throwing', async () => {
    const root = tempDir()

    const result = await inspectPack({ kind: 'file', path: join(root, 'absent.tgz') }, { root })

    expect(result.ok).toBe(false)
    expect(readdirSync(root)).toEqual([])
  })
})

describe('installPack', () => {
  it('lays the pack out atomically and launches from disk', async () => {
    const root = tempDir()
    const file = await buildArchive(goodFiles())
    const seen: ConnectorInstallProgress[] = []

    const result = await installPack(
      { kind: 'file', path: file },
      { root, onProgress: (progress) => seen.push(progress) }
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.pack.id).toBe('acme')
    expect(result.pack.version).toBe('1.2.0')
    expect(result.pack.name).toBe('Acme')
    expect(result.pack.path).toBe(join(root, 'acme', '1.2.0'))
    expect(result.pack.previousVersion).toBeUndefined()
    expect(result.pack.bytes).toBeGreaterThan(0)
    expect(installedLaunch('acme', { root })).toEqual({
      command: 'node',
      args: [join(root, 'acme', '1.2.0', 'index.js')]
    })
    expect(listInstalledPacks({ root }).map((pack) => pack.id)).toEqual(['acme'])
    expect(seen.map((progress) => progress.phase)).toEqual([
      'downloading',
      'verifying',
      'installing',
      'installed'
    ])
    expect(seen.at(-1)?.id).toBe('acme')
    expect(readdirSync(root).filter((entry) => entry.startsWith('.tmp-'))).toEqual([])
  })

  it('keeps one previous version and drops anything older', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.0.0')) }, { root })
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.1.0')) }, { root })
    const third = await installPack(
      { kind: 'file', path: await buildArchive(goodFiles('1.2.0')) },
      { root }
    )

    expect(third.ok).toBe(true)
    if (!third.ok) return
    expect(third.pack.version).toBe('1.2.0')
    expect(third.pack.previousVersion).toBe('1.1.0')
    expect(readdirSync(join(root, 'acme')).sort()).toEqual(['1.1.0', '1.2.0', 'current.json'])
  })

  it('reinstalling the same version keeps the rollback target', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.0.0')) }, { root })
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.1.0')) }, { root })
    const again = await installPack(
      { kind: 'file', path: await buildArchive(goodFiles('1.1.0')) },
      { root }
    )

    expect(again.ok && again.pack.previousVersion).toBe('1.0.0')
  })

  it('unwraps an npm tarball that puts everything under package/', async () => {
    const root = tempDir()
    const file = await buildArchive(goodFiles(), 'package')
    const result = await installPack({ kind: 'file', path: file }, { root })
    expect(result.ok && result.pack.path).toBe(join(root, 'acme', '1.2.0'))
  })

  it('leaves nothing behind when a pack is refused', async () => {
    const root = tempDir()
    const file = await buildArchive({
      'manifest.json': JSON.stringify(manifestFor('acme', '1.2.0')),
      'index.js': '',
      'package.json': JSON.stringify({ dependencies: { zod: '^4' } })
    })
    const seen: ConnectorInstallProgress[] = []

    const result = await installPack(
      { kind: 'file', path: file },
      { root, onProgress: (progress) => seen.push(progress) }
    )

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/declares dependencies/)
    expect(readdirSync(root)).toEqual([])
    expect(seen.at(-1)).toMatchObject({ phase: 'failed' })
    expect(describePack('acme', { root })).toBeUndefined()
  })

  it('refuses an archive whose entries try to escape', async () => {
    const root = tempDir()
    const outside = tempDir()
    const staging = tempDir()
    writeFileSync(join(staging, 'manifest.json'), JSON.stringify(manifestFor('acme', '1.2.0')))
    writeFileSync(join(staging, 'index.js'), '')
    symlinkSync(join(outside, 'stolen.js'), join(staging, 'link.js'))
    const file = join(tempDir(), 'pack.tgz')
    await create({ gzip: true, file, cwd: staging }, ['manifest.json', 'index.js', 'link.js'])

    const result = await installPack({ kind: 'file', path: file }, { root })
    expect(result.ok).toBe(true)
    expect(readdirSync(join(root, 'acme', '1.2.0')).sort()).toEqual(['index.js', 'manifest.json'])
  })

  it('reports a missing file rather than throwing', async () => {
    const result = await installPack(
      { kind: 'file', path: join(tempDir(), 'absent.tgz') },
      { root: tempDir() }
    )
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error).toMatch(/ENOENT|no such file/i)
  })
})

describe('installPack over the network', () => {
  const archiveBytes = async (files: Record<string, string>): Promise<Uint8Array> => {
    const { readFileSync } = await import('node:fs')
    return new Uint8Array(readFileSync(await buildArchive(files)))
  }

  const respond = (body: Uint8Array, headers: Record<string, string> = {}): Response =>
    new Response(body as unknown as BodyInit, { status: 200, headers })

  it('downloads a url pack and reports deduped progress', async () => {
    const root = tempDir()
    const bytes = await archiveBytes(goodFiles())
    const seen: ConnectorInstallProgress[] = []
    const fetchImpl = (async () =>
      respond(bytes, { 'content-length': String(bytes.byteLength) })) as unknown as typeof fetch

    const result = await installPack(
      { kind: 'url', url: 'https://example.test/acme.vorn.tgz' },
      { root, fetchImpl, onProgress: (progress) => seen.push(progress) }
    )

    expect(result.ok).toBe(true)
    const percents = seen
      .filter((progress) => progress.phase === 'downloading')
      .map((progress) => progress.percent)
    expect(percents).toEqual([...new Set(percents)])
    expect(percents.at(-1)).toBe(100)
  })

  it('refuses a download that does not match the published checksum', async () => {
    const bytes = await archiveBytes(goodFiles())
    const fetchImpl = (async () => respond(bytes)) as unknown as typeof fetch
    const result = await installPack(
      { kind: 'url', url: 'https://example.test/acme.vorn.tgz', sha256: 'deadbeef' },
      { root: tempDir(), fetchImpl }
    )
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error).toMatch(/does not match the checksum/)
  })

  it('accepts a download that matches the published checksum', async () => {
    const { createHash } = await import('node:crypto')
    const bytes = await archiveBytes(goodFiles())
    const sha256 = createHash('sha256').update(bytes).digest('hex')
    const fetchImpl = (async () => respond(bytes)) as unknown as typeof fetch
    const result = await installPack(
      { kind: 'url', url: 'https://example.test/acme.vorn.tgz', sha256 },
      { root: tempDir(), fetchImpl }
    )
    expect(result.ok).toBe(true)
  })

  it('reports an http failure rather than installing nothing quietly', async () => {
    const fetchImpl = (async () => new Response('', { status: 404 })) as unknown as typeof fetch
    const result = await installPack(
      { kind: 'url', url: 'https://example.test/gone.tgz' },
      { root: tempDir(), fetchImpl }
    )
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error).toMatch(/HTTP 404/)
  })

  it('resolves an npm package to its published tarball', async () => {
    const bytes = await archiveBytes(goodFiles())
    const asked: string[] = []
    const fetchImpl = (async (url: string) => {
      asked.push(url)
      return url.endsWith('/latest')
        ? Response.json({ dist: { tarball: 'https://registry.test/acme.tgz' } })
        : respond(bytes)
    }) as unknown as typeof fetch

    const result = await installPack(
      { kind: 'npm', packageName: '@vornrun/connector-acme' },
      { root: tempDir(), fetchImpl }
    )

    expect(result.ok).toBe(true)
    expect(asked[0]).toContain('@vornrun%2Fconnector-acme/latest')
    expect(asked[1]).toBe('https://registry.test/acme.tgz')
  })

  it('says so when an npm package publishes no tarball', async () => {
    const fetchImpl = (async () => Response.json({ dist: {} })) as unknown as typeof fetch
    const result = await installPack(
      { kind: 'npm', packageName: 'acme' },
      { root: tempDir(), fetchImpl }
    )
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error).toMatch(/published no tarball/)
  })

  it('stops a download that grows past the size ceiling', async () => {
    const oversize = new Uint8Array(MAX_PACK_BYTES + 1024)
    const fetchImpl = (async () => respond(oversize)) as unknown as typeof fetch
    const result = await installPack(
      { kind: 'url', url: 'https://example.test/huge.tgz' },
      { root: tempDir(), fetchImpl }
    )
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error).toMatch(/installs at most 8 MB/)
  })
})

describe('rollbackPack and removePack', () => {
  it('swaps back to the version kept behind the current one', async () => {
    const root = tempDir()
    const changed: string[] = []
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.0.0')) }, { root })
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.1.0')) }, { root })

    const rolled = await rollbackPack('acme', { root, onChanged: (id) => void changed.push(id) })

    expect(rolled.ok).toBe(true)
    if (!rolled.ok) return
    expect(rolled.pack.version).toBe('1.0.0')
    expect(rolled.pack.previousVersion).toBe('1.1.0')
    expect(installedLaunch('acme', { root })?.args[0]).toBe(join(root, 'acme', '1.0.0', 'index.js'))
    expect(changed).toEqual(['acme'])
  })

  it('refuses to roll back what has only ever been installed once', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.0.0')) }, { root })
    const rolled = await rollbackPack('acme', { root })
    expect(rolled.ok).toBe(false)
    if (!rolled.ok) expect(rolled.error).toMatch(/no earlier version/)
  })

  it('refuses to roll back to a version no longer on disk', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.0.0')) }, { root })
    await installPack({ kind: 'file', path: await buildArchive(goodFiles('1.1.0')) }, { root })
    rmSync(join(root, 'acme', '1.0.0'), { recursive: true, force: true })
    const rolled = await rollbackPack('acme', { root })
    expect(rolled.ok).toBe(false)
    if (!rolled.ok) expect(rolled.error).toMatch(/no longer on disk/)
  })

  it('removes an installed pack and reports one that is not there', async () => {
    const root = tempDir()
    const changed: string[] = []
    await installPack({ kind: 'file', path: await buildArchive(goodFiles()) }, { root })

    expect(await removePack('acme', { root, onChanged: (id) => void changed.push(id) })).toEqual({
      ok: true
    })
    expect(changed).toEqual(['acme'])
    expect(listInstalledPacks({ root })).toEqual([])
    expect(installedLaunch('acme', { root })).toBeUndefined()
    expect(await removePack('acme', { root })).toMatchObject({ ok: false })
  })

  it('refuses an id that could walk out of the connectors directory', async () => {
    const root = tempDir()
    expect(await removePack('../../etc', { root })).toMatchObject({ ok: false })
    expect(await rollbackPack('../../etc', { root })).toMatchObject({ ok: false })
    expect(describePack('../../etc', { root })).toBeUndefined()
    expect(installedLaunch('../../etc', { root })).toBeUndefined()
  })

  it('treats a directory whose manifest went missing as not installed', async () => {
    const root = tempDir()
    await installPack({ kind: 'file', path: await buildArchive(goodFiles()) }, { root })
    rmSync(join(root, 'acme', '1.2.0', 'manifest.json'), { force: true })
    expect(describePack('acme', { root })).toBeUndefined()
    expect(listInstalledPacks({ root })).toEqual([])
  })

  it('lists nothing when the connectors directory has never been made', () => {
    expect(listInstalledPacks({ root: join(tempDir(), 'absent') })).toEqual([])
  })
})
