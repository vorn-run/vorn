import { useCallback, useEffect, useRef, useState } from 'react'
import { browserPartition, type ArtifactComment } from '../../../shared/types'
import { useArtifact } from '../../hooks/useArtifact'
import { useQuoteComments } from '../../hooks/useQuoteComments'
import { ArtifactRail } from '../browser/ArtifactRail'
import { CommentPopover } from '../browser/CommentPopover'

const NONE: ArtifactComment[] = []

/** The registry key the gate's guest is driven under; no session is ever named this. */
export const GATE_GUEST = 'gate-review'

interface WebviewElement extends HTMLElement {
  getWebContentsId(): number
}

interface Props {
  runId: string
  nodeId: string
  round: number
  label: string
  commenting: boolean
  /** Told the drafts as they change, so Request changes can carry them. */
  onDrafts: (drafts: ArtifactComment[]) => void
  /** Told when the gate has no page kept for comments, so the host shows the plain one. */
  onUnavailable: () => void
}

/** A gate's review page in a guest Vorn can read, with the same comments as an artifact in the pane. */
export function GateArtifactView({
  runId,
  nodeId,
  round,
  label,
  commenting,
  onDrafts,
  onUnavailable
}: Props): React.JSX.Element {
  const [shown, setShown] = useState<{ id: string; version: number; url: string } | null>(null)
  const [loadTick, setLoadTick] = useState(0)
  const [failed, setFailed] = useState<string | null>(null)
  const viewRef = useRef<WebviewElement | null>(null)
  const areaRef = useRef<HTMLDivElement | null>(null)
  const { state, refresh } = useArtifact(shown?.id)

  useEffect(() => {
    let live = true
    void window.api
      .artifactForGate(runId, nodeId)
      .then((found) => {
        if (!live) return
        if (found) setShown({ id: found.artifact.id, version: found.version, url: found.url })
        else onUnavailable()
      })
      .catch(() => live && onUnavailable())
    return () => {
      live = false
    }
  }, [runId, nodeId, round, onUnavailable])

  useEffect(() => {
    const view = viewRef.current
    if (!view || !shown) return
    const onReady = (): void => {
      try {
        window.api.attachBrowser(GATE_GUEST, view.getWebContentsId())
      } catch {
        return
      }
      setLoadTick((t) => t + 1)
    }
    view.addEventListener('dom-ready', onReady)
    return () => {
      view.removeEventListener('dom-ready', onReady)
      window.api.detachBrowser(GATE_GUEST)
    }
  }, [shown])

  const onError = useCallback((message: string) => setFailed(message), [])
  const comments = useQuoteComments({
    guestKey: GATE_GUEST,
    artifact: shown ?? undefined,
    comments: state?.comments ?? NONE,
    attached: loadTick > 0,
    enabled: commenting && loadTick > 0,
    watching: true,
    loadTick,
    area: areaRef,
    onSaved: refresh,
    onError
  })

  useEffect(() => onDrafts(comments.drafts), [comments.drafts, onDrafts])

  if (!shown) {
    return (
      <div className="flex-1 flex items-center justify-center text-[12px] text-ink-faint">
        Opening the review page…
      </div>
    )
  }

  return (
    <div className="flex-1 min-h-0 flex">
      <div ref={areaRef} className="flex-1 min-w-0 relative bg-white">
        <webview
          ref={viewRef as unknown as React.Ref<HTMLElement>}
          src={shown.url}
          partition={browserPartition(GATE_GUEST)}
          title={`Review page for ${label}`}
          className="absolute inset-0 w-full h-full"
        />
        {comments.pending && (
          <CommentPopover
            key={comments.pending.label}
            quote={comments.pending.label}
            at={comments.pending.at}
            onAdd={comments.addComment}
            onCancel={comments.dropSelection}
          />
        )}
        {failed && (
          <div className="absolute left-2 bottom-2 px-2 py-1 rounded text-[11px] text-danger bg-surface-overlay">
            {failed}
          </div>
        )}
      </div>
      {commenting && (
        <ArtifactRail
          heading="Comments"
          drafts={comments.drafts}
          sent={comments.sentBatch}
          version={shown.version}
          found={comments.found}
          agent="the run"
          queued={false}
          sending={false}
          onEdit={comments.editComment}
          onDelete={comments.deleteComment}
          onReveal={comments.revealComment}
          onAddNote={comments.addNote}
          hint="Select words on the page to comment on them. Request changes carries them back."
        />
      )}
    </div>
  )
}
