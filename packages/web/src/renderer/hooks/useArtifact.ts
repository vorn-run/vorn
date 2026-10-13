import { useCallback, useEffect, useState } from 'react'
import type { Artifact, ArtifactComment, ArtifactVersion } from '../../shared/types'

export interface ArtifactState {
  artifact: Artifact
  versions: ArtifactVersion[]
  comments: ArtifactComment[]
  queued: boolean
}

/** An artifact with its versions and comments, kept current as it is republished or commented on. */
export function useArtifact(artifactId: string | undefined): {
  state: ArtifactState | null
  refresh: () => void
} {
  const [state, setState] = useState<ArtifactState | null>(null)
  const [tick, setTick] = useState(0)
  const refresh = useCallback(() => setTick((t) => t + 1), [])

  useEffect(() => {
    if (!artifactId) return
    let stale = false
    void window.api
      .getArtifact(artifactId)
      .then((next) => {
        if (!stale) setState(next)
      })
      .catch(() => {
        if (!stale) setState(null)
      })
    return () => {
      stale = true
    }
  }, [artifactId, tick])

  useEffect(() => {
    if (!artifactId) return
    const offPublished = window.api.onArtifactPublished(({ artifact }) => {
      if (artifact.id === artifactId) refresh()
    })
    const offComments = window.api.onArtifactCommentsChanged(({ artifactId: id }) => {
      if (id === artifactId) refresh()
    })
    return () => {
      offPublished()
      offComments()
    }
  }, [artifactId, refresh])

  return { state: state?.artifact.id === artifactId ? state : null, refresh }
}
