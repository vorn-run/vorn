import { deleteArtifactsUpdatedBefore, listArtifactIds } from '../database'
import { sweepArtifactBodies } from './bodies'

/** How long an artifact nobody has touched is kept. */
export const ARTIFACT_RETENTION_DAYS = 90

/** Forget artifacts untouched for the retention period, and any files left without a row. */
export function sweepArtifacts(dataDir: string, now = Date.now()): void {
  const cutoff = new Date(now - ARTIFACT_RETENTION_DAYS * 24 * 60 * 60 * 1000).toISOString()
  deleteArtifactsUpdatedBefore(cutoff)
  sweepArtifactBodies(dataDir, listArtifactIds())
}
