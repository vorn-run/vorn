import {
  deleteArtifactsUpdatedBefore,
  getArtifact,
  getArtifactToken,
  listArtifactIds
} from '../database'
import { sameToken } from '../workflows/gate-views'
import { readVersionBody, sweepArtifactBodies } from './bodies'
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
