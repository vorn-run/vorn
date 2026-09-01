/** Connectors installed as verified files, so launching one resolves nothing. */
import { createHash } from 'node:crypto'
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { extract } from 'tar'
import type {
  ConnectorInstallProgress,
  ConnectorPackResult,
  ConnectorPackSource,
  InstalledConnectorPack,
  SdkConnectorManifest
} from '@vornrun/shared/types'
import { getDataDir } from '../database'
import { toManifest } from './sdk-probe'
import log from '../logger'

/** Largest archive Vorn will install, matched by the SDK's own pack gate. */
export const MAX_PACK_BYTES = 8 * 1024 * 1024

/** A small archive can still unpack to something enormous. */
export const MAX_UNPACKED_BYTES = 32 * 1024 * 1024

/** A slow mirror must not wedge an install forever. */
const DOWNLOAD_TIMEOUT_MS = 60_000

const ENTRY_FILE = 'index.js'
const MANIFEST_FILE = 'manifest.json'
const CURRENT_FILE = 'current.json'

interface CurrentPack {
  version: string
  previousVersion?: string
  installedAt: number
}

/** The impure edges, defaulted for production and supplied by tests. */
export interface PackOptions {
  root?: string
  fetchImpl?: typeof fetch
  onProgress?(progress: ConnectorInstallProgress): void
  /** Called after a mutation so live children of affected connections restart. */
  onChanged?(id: string): void | Promise<void>
}

export function packsRoot(options: PackOptions = {}): string {
  return options.root ?? join(getDataDir(), 'connectors')
}

function packDir(id: string, options: PackOptions): string {
  return join(packsRoot(options), id)
}

function readCurrent(id: string, options: PackOptions): CurrentPack | undefined {
  try {
    const parsed = JSON.parse(
      readFileSync(join(packDir(id, options), CURRENT_FILE), 'utf8')
    ) as CurrentPack
    return typeof parsed?.version === 'string' && parsed.version !== '' ? parsed : undefined
  } catch {
    // Never installed, or a pointer written by a version that shaped it differently.
    return undefined
  }
}

function writeCurrent(id: string, current: CurrentPack, options: PackOptions): void {
  const dir = packDir(id, options)
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, CURRENT_FILE), JSON.stringify(current, null, 2))
}

function directoryBytes(dir: string): number {
  let total = 0
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name)
    total += entry.isDirectory() ? directoryBytes(full) : statSync(full).size
  }
  return total
}

/** Ids are directory names, so anything that could traverse is not one. */
function isSafeId(id: string): boolean {
  return /^[a-z0-9][a-z0-9-]{0,63}$/i.test(id)
}

/** Checked again here because a manifest's id becomes a path segment. */
function requireSafeId(id: string): void {
  if (!isSafeId(id)) throw new Error(`"${id}" is not a usable connector id`)
}

function readManifest(dir: string): SdkConnectorManifest {
  let payload: unknown
  try {
    payload = JSON.parse(readFileSync(join(dir, MANIFEST_FILE), 'utf8'))
  } catch {
    throw new Error('The pack has no readable manifest.json')
  }
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    throw new Error('The pack has no readable manifest.json')
  }
  return toManifest(payload as Record<string, unknown>)
}

/** Every file in a directory tree, as paths relative to its root. */
function walk(dir: string, prefix = ''): string[] {
  const found: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name
    if (entry.isDirectory()) found.push(...walk(join(dir, entry.name), rel))
    else found.push(rel)
  }
  return found
}

/** Refuse anything needing an install step, running code, or unpacking too large. */
export function verifyPackDir(dir: string): SdkConnectorManifest {
  const files = walk(dir)
  if (!files.includes(MANIFEST_FILE)) throw new Error('The pack has no manifest.json')

  const scripts = files.filter((file) => file.endsWith('.js') || file.endsWith('.mjs'))
  if (scripts.length === 0) throw new Error('The pack has no entry to run')
  if (scripts.length > 1 || scripts[0] !== ENTRY_FILE) {
    throw new Error(`The pack must carry exactly one ${ENTRY_FILE}, not ${scripts.join(', ')}`)
  }

  for (const file of files.filter((entry) => entry.endsWith('package.json'))) {
    let pkg: Record<string, unknown>
    try {
      pkg = JSON.parse(readFileSync(join(dir, file), 'utf8')) as Record<string, unknown>
    } catch {
      continue
    }
    const dependencies = pkg.dependencies
    if (
      dependencies &&
      typeof dependencies === 'object' &&
      Object.keys(dependencies as object).length > 0
    ) {
      throw new Error(
        `${file} declares dependencies; a pack must carry everything it needs so it can launch with no registry`
      )
    }
    if (pkg.scripts && typeof pkg.scripts === 'object' && Object.keys(pkg.scripts).length > 0) {
      throw new Error(
        `${file} declares scripts; a pack is installed by copying files, never by running them`
      )
    }
  }

  const bytes = directoryBytes(dir)
  if (bytes > MAX_UNPACKED_BYTES) {
    throw new Error(
      `The pack unpacks to ${Math.round(bytes / 1024 / 1024)} MB; Vorn installs at most ${MAX_UNPACKED_BYTES / 1024 / 1024} MB`
    )
  }

  return readManifest(dir)
}

/** Stated rather than inherited from tar: being wrong here writes outside the data dir. */
export function isSafeArchiveEntry(path: string, type: string): boolean {
  if (type !== 'File' && type !== 'Directory') return false
  if (path.startsWith('/') || /^[a-z]:/i.test(path)) return false
  return !path.split(/[/\\]/).includes('..')
}

/** npm tarballs put everything under `package/`; a `.vorn.tgz` does not. */
function packRootOf(dir: string): string {
  if (existsSync(join(dir, MANIFEST_FILE))) return dir
  const entries = readdirSync(dir, { withFileTypes: true })
  const single = entries.length === 1 && entries[0].isDirectory() ? entries[0].name : undefined
  return single ? join(dir, single) : dir
}

async function readSource(
  source: ConnectorPackSource,
  options: PackOptions,
  report: (progress: Omit<ConnectorInstallProgress, 'id'>) => void
): Promise<Buffer> {
  if (source.kind === 'file') {
    const bytes = readFileSync(source.path)
    report({ phase: 'downloading', percent: 100 })
    return bytes
  }

  const get = options.fetchImpl ?? fetch
  const url = source.kind === 'url' ? source.url : await resolveNpmTarball(source.packageName, get)
  const response = await get(url, { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) })
  if (!response.ok) throw new Error(`Downloading the pack failed with HTTP ${response.status}`)

  const total = Number(response.headers.get('content-length')) || 0
  const chunks: Uint8Array[] = []
  let received = 0
  let lastPercent = -1
  for await (const chunk of streamOf(response)) {
    received += chunk.byteLength
    if (received > MAX_PACK_BYTES) throw new Error(sizeMessage(received))
    chunks.push(chunk)
    if (total <= 0) continue
    // Dropping repeats of a rounded percent keeps this to about a hundred pushes.
    const percent = Math.round((received / total) * 100)
    if (percent === lastPercent) continue
    lastPercent = percent
    report({ phase: 'downloading', percent })
  }

  const bytes = Buffer.concat(chunks)
  if (source.kind === 'url' && source.sha256) {
    const digest = createHash('sha256').update(bytes).digest('hex')
    if (digest !== source.sha256.toLowerCase()) {
      throw new Error('The downloaded pack does not match the checksum the catalog published')
    }
  }
  return bytes
}

async function* streamOf(response: Response): AsyncGenerator<Uint8Array> {
  const body = response.body
  if (!body) {
    yield new Uint8Array(await response.arrayBuffer())
    return
  }
  const reader = body.getReader()
  for (;;) {
    const { done, value } = await reader.read()
    if (done) return
    if (value) yield value
  }
}

async function resolveNpmTarball(packageName: string, get: typeof fetch): Promise<string> {
  const response = await get(
    `https://registry.npmjs.org/${encodeURIComponent(packageName).replace('%40', '@')}/latest`,
    { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) }
  )
  if (!response.ok) {
    throw new Error(`${packageName} could not be looked up (HTTP ${response.status})`)
  }
  const metadata = (await response.json()) as { dist?: { tarball?: unknown } }
  const tarball = metadata?.dist?.tarball
  if (typeof tarball !== 'string' || tarball === '') {
    throw new Error(`${packageName} published no tarball to install`)
  }
  return tarball
}

function sizeMessage(bytes: number): string {
  return `The pack is ${Math.round(bytes / 1024)} KB; Vorn installs at most ${MAX_PACK_BYTES / 1024 / 1024} MB`
}

function describeSource(source: ConnectorPackSource): string {
  if (source.kind === 'npm') return source.packageName
  return source.kind === 'file' ? source.path : source.url
}

/** The rename is the commit point; everything before it is in a temporary directory. */
export async function installPack(
  source: ConnectorPackSource,
  options: PackOptions = {}
): Promise<ConnectorPackResult> {
  const label = describeSource(source)
  let id = label
  const report = (progress: Omit<ConnectorInstallProgress, 'id'>): void => {
    options.onProgress?.({ id, ...progress })
  }

  const root = packsRoot(options)
  mkdirSync(root, { recursive: true })
  const staging = join(
    root,
    `.tmp-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
  )

  try {
    const archive = await readSource(source, options, report)
    if (archive.byteLength > MAX_PACK_BYTES) throw new Error(sizeMessage(archive.byteLength))

    report({ phase: 'verifying' })
    mkdirSync(staging, { recursive: true })
    const archivePath = join(staging, 'pack.tgz')
    await writeFile(archivePath, archive)
    const unpacked = join(staging, 'unpacked')
    mkdirSync(unpacked, { recursive: true })
    await extract({
      file: archivePath,
      cwd: unpacked,
      preservePaths: false,
      filter: (path, entry) =>
        isSafeArchiveEntry(path, String((entry as { type?: unknown }).type ?? ''))
    })

    const contents = packRootOf(unpacked)
    const manifest = verifyPackDir(contents)
    requireSafeId(manifest.id)
    id = manifest.id

    report({ phase: 'installing', version: manifest.version })
    const current = readCurrent(manifest.id, options)
    const target = join(packDir(manifest.id, options), manifest.version)
    mkdirSync(packDir(manifest.id, options), { recursive: true })
    rmSync(target, { recursive: true, force: true })
    renameSync(contents, target)

    // One version is kept behind the current one, so a bad update is a click to undo.
    const previousVersion =
      current && current.version !== manifest.version ? current.version : current?.previousVersion
    const installedAt = Date.now()
    writeCurrent(
      manifest.id,
      {
        version: manifest.version,
        ...(previousVersion !== undefined && { previousVersion }),
        installedAt
      },
      options
    )
    pruneVersions(manifest.id, [manifest.version, previousVersion], options)

    await options.onChanged?.(manifest.id)
    const pack = describePack(manifest.id, options)
    if (!pack) throw new Error('The pack was installed but could not be read back')
    log.info(`[packs] installed ${manifest.id}@${manifest.version} from ${source.kind}`)
    report({ phase: 'installed', version: manifest.version })
    return { ok: true, pack }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    log.warn(`[packs] install from ${label} refused: ${message}`)
    report({ phase: 'failed', error: message })
    return { ok: false, error: message }
  } finally {
    rmSync(staging, { recursive: true, force: true })
  }
}

function pruneVersions(id: string, keep: Array<string | undefined>, options: PackOptions): void {
  const dir = packDir(id, options)
  const kept = new Set(keep.filter((version): version is string => version !== undefined))
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (!entry.isDirectory() || kept.has(entry.name)) continue
    rmSync(join(dir, entry.name), { recursive: true, force: true })
  }
}

/** The installed pack for a connector, or undefined when there is none. */
export function describePack(
  id: string,
  options: PackOptions = {}
): InstalledConnectorPack | undefined {
  if (!isSafeId(id)) return undefined
  const current = readCurrent(id, options)
  if (!current) return undefined
  const path = join(packDir(id, options), current.version)
  if (!existsSync(join(path, ENTRY_FILE))) return undefined

  let manifest: SdkConnectorManifest
  try {
    manifest = readManifest(path)
  } catch {
    // The directory outlived its manifest; nothing can describe it, so it is not installed.
    return undefined
  }

  return {
    id,
    name: manifest.name,
    version: current.version,
    ...(manifest.description !== undefined && { description: manifest.description }),
    ...(manifest.icon !== undefined && { icon: manifest.icon }),
    path,
    ...(current.previousVersion !== undefined && { previousVersion: current.previousVersion }),
    installedAt: current.installedAt,
    bytes: directoryBytes(path),
    triggers: manifest.triggers,
    actions: manifest.actions,
    env: manifest.env
  }
}

export function listInstalledPacks(options: PackOptions = {}): InstalledConnectorPack[] {
  const root = packsRoot(options)
  if (!existsSync(root)) return []
  return readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && !entry.name.startsWith('.'))
    .flatMap((entry) => {
      const pack = describePack(entry.name, options)
      return pack ? [pack] : []
    })
    .sort((a, b) => a.name.localeCompare(b.name))
}

/** How to launch an installed connector, with nothing left to resolve. */
export function installedLaunch(
  id: string,
  options: PackOptions = {}
): { command: string; args: string[] } | undefined {
  const pack = describePack(id, options)
  return pack ? { command: 'node', args: [join(pack.path, ENTRY_FILE)] } : undefined
}

/** Swap back to the one version kept behind the current one. */
export async function rollbackPack(
  id: string,
  options: PackOptions = {}
): Promise<ConnectorPackResult> {
  if (!isSafeId(id)) return { ok: false, error: `"${id}" is not a usable connector id` }
  const current = readCurrent(id, options)
  if (!current?.previousVersion) {
    return { ok: false, error: 'There is no earlier version to roll back to' }
  }
  if (!existsSync(join(packDir(id, options), current.previousVersion, ENTRY_FILE))) {
    return { ok: false, error: `Version ${current.previousVersion} is no longer on disk` }
  }

  writeCurrent(
    id,
    { version: current.previousVersion, previousVersion: current.version, installedAt: Date.now() },
    options
  )
  await options.onChanged?.(id)
  const pack = describePack(id, options)
  if (!pack) return { ok: false, error: 'The earlier version could not be read back' }
  log.info(`[packs] rolled ${id} back to ${pack.version}`)
  return { ok: true, pack }
}

export async function removePack(
  id: string,
  options: PackOptions = {}
): Promise<{ ok: boolean; error?: string }> {
  if (!isSafeId(id)) return { ok: false, error: `"${id}" is not a usable connector id` }
  const dir = packDir(id, options)
  if (!existsSync(dir)) return { ok: false, error: `${id} is not installed` }
  rmSync(dir, { recursive: true, force: true })
  await options.onChanged?.(id)
  log.info(`[packs] removed ${id}`)
  return { ok: true }
}
