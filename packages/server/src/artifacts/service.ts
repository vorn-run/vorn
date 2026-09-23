import fs from 'node:fs'
import path from 'node:path'
import type { Artifact, ArtifactKind, ArtifactVersion } from '@vornrun/shared/types'
import {
  addArtifactVersion,
  deleteArtifactsUpdatedBefore,
  getArtifact,
  getArtifactToken,
  insertArtifact,
  listArtifactComments,
  listArtifactIds,
  renameArtifact,
  unansweredBatchId
} from '../database'
import { sameToken } from '../workflows/gate-views'
import {
  ARTIFACT_MAX_BYTES,
  readVersionBody,
  sweepArtifactBodies,
  tooBigMessage,
  writeVersionBody
} from './bodies'
import { renderDocPage } from './doc-page'

/** How long an artifact nobody has touched is kept. */
export const ARTIFACT_RETENTION_DAYS = 90

/** Forget artifacts untouched for the retention period, and any files left without a row. */
export function sweepArtifacts(dataDir: string, now = Date.now()): void {
  const cutoff = new Date(now - ARTIFACT_RETENTION_DAYS * 24 * 60 * 60 * 1000).toISOString()
  deleteArtifactsUpdatedBefore(cutoff)
  sweepArtifactBodies(dataDir, listArtifactIds())
}

/** The HTML for one version, or null unless the token is this artifact's and the version exists. */
export function artifactPage(
  dataDir: string,
  artifactId: string,
  version: number,
  token: string
): string | null {
  const expected = getArtifactToken(artifactId)
  const artifact = getArtifact(artifactId)
  if (!expected || !artifact || !sameToken(expected, token)) return null
  if (version > artifact.latestVersion) return null
  const body = readVersionBody(dataDir, artifactId, version, artifact.kind)
  if (body === null) return null
  return artifact.kind === 'doc' ? renderDocPage(artifact.title, body) : body
}

/** The session publishing, as far as publishing needs to know it. */
export interface PublishingSession {
  id: string
  projectName: string | null
  /** The directory a `file` must sit inside: the worktree, else the project. */
  root: string | null
}

export interface PublishRequest {
  kind: ArtifactKind
  title: string
  file?: string
  content?: string
  artifactId?: string
}

export interface PublishOutcome {
  artifact: Artifact
  version: ArtifactVersion
  /** Where the version is served, relative to the server's origin. */
  path: string
  /** How many comments this version answers. */
  answered: number
}

const EXTENSIONS: Record<ArtifactKind, RegExp> = {
  page: /\.html?$/i,
  design: /\.html?$/i,
  doc: /\.(md|markdown)$/i
}

export function artifactPath(artifactId: string, version: number, token: string): string {
  return `/artifact/${encodeURIComponent(artifactId)}/${version}?t=${encodeURIComponent(token)}`
}

/** Whether a session may see this artifact: it published it, or it works in the same project. */
export function canSeeArtifact(
  artifact: Artifact,
  session: Pick<PublishingSession, 'id' | 'projectName'>
): boolean {
  if (artifact.sessionId === session.id) return true
  return artifact.projectName !== null && artifact.projectName === session.projectName
}

function readInsideRoot(root: string | null, file: string, kind: ArtifactKind): string {
  if (!root)
    throw new Error('This session has no project folder, so publish `content` instead of a file.')
  if (!EXTENSIONS[kind].test(file)) {
    throw new Error(
      kind === 'doc'
        ? 'A doc is published from a .md file.'
        : 'A page or design is published from a .html file.'
    )
  }
  const realRoot = fs.realpathSync(root)
  let real: string
  try {
    real = fs.realpathSync(path.resolve(root, file))
  } catch {
    throw new Error(`No such file: ${file}`)
  }
  if (real !== realRoot && !real.startsWith(realRoot + path.sep)) {
    throw new Error(`Refusing to publish ${file}: it is outside this session's folder.`)
  }
  const size = fs.statSync(real).size
  if (size > ARTIFACT_MAX_BYTES) throw new Error(tooBigMessage(size))
  return fs.readFileSync(real, 'utf8')
}

/** Publish a first version, or the next version of an artifact this session can see. */
export function publishArtifact(
  dataDir: string,
  session: PublishingSession,
  request: PublishRequest
): PublishOutcome {
  const title = request.title.trim()
  if (!title) throw new Error('An artifact needs a title.')
  if ((request.file === undefined) === (request.content === undefined)) {
    throw new Error('Publish either a file or content, not both and not neither.')
  }
  const body =
    request.file !== undefined
      ? readInsideRoot(session.root, request.file, request.kind)
      : request.content!
  if (!body.trim()) throw new Error('The artifact is empty.')
  const bytes = Buffer.byteLength(body)
  if (bytes > ARTIFACT_MAX_BYTES) throw new Error(tooBigMessage(bytes))

  let artifact: Artifact
  let token: string
  let answers: string | undefined
  if (request.artifactId) {
    const existing = getArtifact(request.artifactId)
    if (!existing || !canSeeArtifact(existing, session)) {
      throw new Error(`No artifact ${request.artifactId} in this session or project.`)
    }
    if (existing.kind !== request.kind) {
      throw new Error(
        `Artifact ${existing.id} is a ${existing.kind}; it cannot become a ${request.kind}.`
      )
    }
    if (existing.title !== title) renameArtifact(existing.id, title)
    artifact = existing
    token = getArtifactToken(existing.id)!
    answers = unansweredBatchId(existing.id)
  } else {
    ;({ artifact, token } = insertArtifact({
      kind: request.kind,
      title,
      sessionId: session.id,
      projectName: session.projectName
    }))
  }

  const version = addArtifactVersion(artifact.id, 'agent', answers)
  writeVersionBody(dataDir, artifact.id, version.version, artifact.kind, body)
  return {
    artifact: getArtifact(artifact.id)!,
    version,
    path: artifactPath(artifact.id, version.version, token),
    answered: answers ? listArtifactComments(artifact.id, { batchId: answers }).length : 0
  }
}
