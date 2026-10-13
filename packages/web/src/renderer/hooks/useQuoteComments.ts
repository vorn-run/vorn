import { useCallback, useEffect, useMemo, useState } from 'react'
import type { ArtifactAnchor, ArtifactComment } from '../../shared/types'
import { latestSentBatch, marksFor, placePopover } from '../lib/artifact-comments'

/** A comment being written: what it is anchored to, how the popover names that, and where it opens. */
export interface PendingComment {
  anchor: ArtifactAnchor
  label: string
  at: { x: number; y: number }
}

interface Options {
  /** The browser registry key of the guest showing the artifact. */
  guestKey: string
  artifact: { id: string; version: number } | undefined
  comments: ArtifactComment[]
  /** Whether the guest can be painted; with comments off its highlights are cleared. */
  attached: boolean
  /** Whether comments are drawn and taken at all. */
  enabled: boolean
  /** Whether a selection on the page opens a comment. */
  watching: boolean
  /** Bumped when the page loads, so highlights are painted again. */
  loadTick: number
  area: React.RefObject<HTMLDivElement | null>
  onSaved: () => void
  onError: (message: string) => void
}

/** Comments on the words of an artifact in a guest: the selection, the highlights and the drafts. */
export function useQuoteComments({
  guestKey,
  artifact,
  comments,
  attached,
  enabled,
  watching,
  loadTick,
  area,
  onSaved,
  onError
}: Options) {
  const [pending, setPending] = useState<PendingComment | null>(null)
  const [found, setFound] = useState<Record<string, boolean>>({})
  const [focusId, setFocusId] = useState<string | null>(null)

  const drafts = useMemo(() => comments.filter((c) => c.state === 'draft'), [comments])
  const sentBatch = useMemo(() => latestSentBatch(comments), [comments])
  const marksKey = JSON.stringify(enabled ? marksFor(drafts, sentBatch, focusId) : [])

  // Watch the page for a selection; the page itself is never given a way to call in.
  useEffect(() => {
    if (!enabled || !watching || pending) return
    let stale = false
    let last = ''
    const timer = window.setInterval(() => {
      void window.api
        .artifactSelection(guestKey)
        .then((sel) => {
          const rect = area.current?.getBoundingClientRect()
          // Open only once the same words are read twice, so a drag still in progress is left alone.
          const seen = last
          last = sel ? JSON.stringify(sel.anchor) : ''
          if (stale || !sel || !rect || last !== seen) return
          setPending({
            anchor: sel.anchor,
            label: sel.anchor.quote,
            at: placePopover(sel.rect, rect)
          })
        })
        .catch(() => {})
    }, 400)
    return () => {
      stale = true
      window.clearInterval(timer)
    }
  }, [enabled, watching, pending, guestKey, area])

  useEffect(() => {
    if (!attached) return
    let stale = false
    void window.api
      .paintArtifactMarks(guestKey, JSON.parse(marksKey))
      .then((r) => {
        if (!stale) setFound(r.found)
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [marksKey, loadTick, attached, guestKey])

  const dropSelection = useCallback(() => {
    setPending(null)
    void window.api.clearArtifactSelection(guestKey).catch(() => {})
  }, [guestKey])

  const addComment = useCallback(
    (body: string) => {
      if (!artifact || !pending) return
      void window.api
        .saveArtifactComment({
          artifactId: artifact.id,
          version: artifact.version,
          anchor: pending.anchor,
          body
        })
        .then(onSaved)
        .catch(() => onError('Could not save the comment'))
      dropSelection()
    },
    [artifact, pending, onSaved, onError, dropSelection]
  )

  const addNote = useCallback(
    (body: string) => {
      if (!artifact) return
      void window.api
        .saveArtifactComment({
          artifactId: artifact.id,
          version: artifact.version,
          anchor: null,
          body
        })
        .then(onSaved)
        .catch(() => onError('Could not save the note'))
    },
    [artifact, onSaved, onError]
  )

  const editComment = useCallback(
    (id: string, body: string) =>
      void window.api
        .updateArtifactComment({ commentId: id, body })
        .then(onSaved)
        .catch(() => {}),
    [onSaved]
  )

  const deleteComment = useCallback(
    (id: string) =>
      void window.api
        .deleteArtifactComment(id)
        .then(onSaved)
        .catch(() => {}),
    [onSaved]
  )

  const revealComment = useCallback(
    (c: ArtifactComment) => {
      setFocusId(c.id)
      if (c.anchor?.kind !== 'quote') return
      const { quote, prefix, suffix } = c.anchor
      void window.api
        .revealArtifactMark(guestKey, { id: c.id, quote, prefix, suffix, state: 'focus' })
        .catch(() => {})
    },
    [guestKey]
  )

  return {
    pending,
    setPending,
    found,
    focusId,
    drafts,
    sentBatch,
    dropSelection,
    addComment,
    addNote,
    editComment,
    deleteComment,
    revealComment
  }
}
