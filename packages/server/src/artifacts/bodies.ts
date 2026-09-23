import fs from 'node:fs'
import path from 'node:path'
import type { ArtifactKind } from '@vornrun/shared/types'

/** Largest version an artifact keeps, the same ceiling as a gate's review page. */
export const ARTIFACT_MAX_BYTES = 5 * 1024 * 1024

const segment = (id: string): string => id.replace(/[^A-Za-z0-9._-]/g, '_')

function artifactDir(dataDir: string, artifactId: string): string {
  return path.join(dataDir, 'artifacts', segment(artifactId))
}

/** A doc is kept as the Markdown it was written in; pages and designs as HTML. */
export function versionFile(
  dataDir: string,
  artifactId: string,
  version: number,
  kind: ArtifactKind
): string {
  return path.join(artifactDir(dataDir, artifactId), `${version}.${kind === 'doc' ? 'md' : 'html'}`)
}

export function tooBigMessage(bytes: number): string {
  return `The artifact is ${(bytes / 1024 / 1024).toFixed(1)} MB; the limit is 5 MB.`
}

export function writeVersionBody(
  dataDir: string,
  artifactId: string,
  version: number,
  kind: ArtifactKind,
  body: string
): void {
  const file = versionFile(dataDir, artifactId, version, kind)
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, body)
}

export function readVersionBody(
  dataDir: string,
  artifactId: string,
  version: number,
  kind: ArtifactKind
): string | null {
  try {
    return fs.readFileSync(versionFile(dataDir, artifactId, version, kind), 'utf8')
  } catch {
    return null
  }
}

export function removeArtifactBodies(dataDir: string, artifactId: string): void {
  fs.rmSync(artifactDir(dataDir, artifactId), { recursive: true, force: true })
}

/** Drop the files of artifacts no longer kept. */
export function sweepArtifactBodies(dataDir: string, keptIds: Iterable<string>): void {
  let dirs: string[]
  try {
    dirs = fs.readdirSync(path.join(dataDir, 'artifacts'))
  } catch {
    return
  }
  const kept = new Set([...keptIds].map(segment))
  for (const dir of dirs) {
    if (!kept.has(dir))
      fs.rmSync(path.join(dataDir, 'artifacts', dir), { recursive: true, force: true })
  }
}
