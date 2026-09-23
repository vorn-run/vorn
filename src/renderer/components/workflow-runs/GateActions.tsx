import { useMemo, useState } from 'react'
import { Check, Eye, MessageSquare, Pencil, RotateCcw, X } from 'lucide-react'
import { canRequestChanges, gateMaxRounds } from '@vornrun/shared/workflow-graph'
import { isRecordList, toItemList } from '@vornrun/shared/item-list'
import type {
  ApprovalConfig,
  GateComment,
  GateDecision,
  NodeExecutionState,
  WorkflowNode
} from '../../../shared/types'
import { GATE_APPROVE, GATE_NEUTRAL, GATE_REJECT } from '../../lib/gate-affordance'
import { toast } from '../Toast'
import { GateJsonEditor, type GateDraft } from './GateJsonEditor'
import { useReportDraft } from '../../hooks/useReportDraft'

/** Ask the server to settle a gate; says so when the gate would not take the answer. */
async function answerGate(
  runId: string,
  nodeId: string,
  decision: GateDecision,
  comment?: string,
  edited?: string,
  comments?: GateComment[]
): Promise<boolean> {
  const result = await window.api.resolveWorkflowGate({
    runId,
    nodeId,
    decision,
    ...(comment && { comment }),
    ...(edited && { edited }),
    ...(comments?.length && { comments })
  })
  if (result?.accepted !== false) return true
  toast.error(
    result.reason ??
      (decision === 'changes'
        ? 'This gate takes no more changes. Approve or reject it.'
        : 'The gate did not take that answer.')
  )
  return false
}

/** What the gate asks, as it was filled in, and the last change the reviewer asked for. */
export function GateAsk({ state, config }: { state: NodeExecutionState; config?: ApprovalConfig }) {
  const message = state.message ?? config?.message
  const asked = state.feedback?.filter((f) => f.decision === 'changes').at(-1)?.comment
  return (
    <>
      {message ? (
        <div className="max-h-48 overflow-y-auto rounded-md border border-white/[0.07] bg-surface-base px-2.5 py-2 text-[12px] leading-[1.5] text-ink whitespace-pre-line">
          {message}
        </div>
      ) : (
        <div className="text-[11.5px] text-bronzo">Waiting for approval.</div>
      )}
      {asked && (
        <div className="flex gap-1.5 text-[11.5px] leading-[1.45] text-ink-secondary">
          <span className="shrink-0 text-ink-faint">You asked</span>
          <span className="line-clamp-2">{asked}</span>
        </div>
      )}
    </>
  )
}

interface ComposerProps {
  runId: string
  state: NodeExecutionState
  config?: ApprovalConfig
  nodes: WorkflowNode[]
  kind: 'changes' | 'reject'
  /** A rewrite the reviewer made before answering; sent back with the work, discarded on a reject. */
  edited?: string
  /** Comments left on the review page; a request for changes carries them and then needs no note. */
  comments?: GateComment[]
  onDone: () => void
  large?: boolean
}

/** The comment a request for changes needs, or the note a rejection may carry. */
export function GateComposer({
  runId,
  state,
  config,
  nodes,
  kind,
  edited,
  comments,
  onDone,
  large
}: ComposerProps) {
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const pinned = kind === 'changes' ? (comments?.length ?? 0) : 0
  const needsText = kind === 'changes' && pinned === 0
  const from = nodes.find((n) => n.id === config?.feedback?.from)
  const size = large ? 'px-3.5 py-2 text-[12.5px]' : 'px-2 py-1 text-[11px]'
  const icon = large ? 13 : 11

  const submit = async (): Promise<void> => {
    if (sending || (needsText && !text.trim())) return
    setSending(true)
    try {
      const carried = kind === 'changes' ? edited : undefined
      const carriedComments = kind === 'changes' ? comments : undefined
      const note = text.trim() || undefined
      if (await answerGate(runId, state.nodeId, kind, note, carried, carriedComments)) onDone()
    } finally {
      setSending(false)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <textarea
        id={`gate-${kind}-${state.nodeId}`}
        aria-label={kind === 'changes' ? 'What should change' : 'Why the run is rejected'}
        autoFocus
        rows={3}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation()
            onDone()
          } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault()
            void submit()
          }
        }}
        placeholder={
          needsText
            ? 'What should change?'
            : pinned
              ? 'Anything else to change (optional)'
              : 'Why, if it should be kept with the run (optional)'
        }
        className="w-full px-2.5 py-2 bg-white/[0.03] border border-white/[0.2] rounded-md
                   text-[12.5px] leading-[1.5] text-gray-200 placeholder:text-gray-600
                   focus:outline-none resize-none"
      />
      <div className="text-[11px] leading-[1.45] text-ink-faint">
        {kind === 'changes' ? (
          <>
            Runs again from{' '}
            <span className="text-ink-secondary">{from?.label || 'the chosen step'}</span> with{' '}
            {pinned
              ? `your ${pinned} comment${pinned === 1 ? '' : 's'} on the page`
              : 'your comment'}
            , then asks you. Round {(state.round ?? 1) + 1} of {gateMaxRounds(config?.feedback)}.
          </>
        ) : (
          'Ends the run. A note is kept as its reason.'
        )}
        {edited &&
          (kind === 'changes' ? ' Your edit goes back with it.' : ' Your edit is discarded.')}
      </div>
      <div className="flex items-center justify-end gap-1.5">
        <button
          type="button"
          onClick={onDone}
          className={`${size} text-ink-faint hover:text-ink-secondary transition-colors`}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={sending || (needsText && !text.trim())}
          onClick={() => void submit()}
          className={`flex items-center gap-1 ${size} ${kind === 'changes' ? GATE_APPROVE : GATE_REJECT} disabled:opacity-40 disabled:pointer-events-none`}
        >
          {kind === 'changes' ? (
            <RotateCcw size={icon} strokeWidth={2} />
          ) : (
            <X size={icon} strokeWidth={2.5} />
          )}
          {kind === 'changes' ? 'Send back' : 'Reject run'}
        </button>
      </div>
    </div>
  )
}

/**
 * The gate's text, for the reviewer to rewrite before they answer.
 *
 * A list of records opens as a table, because trimming and correcting rows is
 * what a reviewer does with one, and hand-editing JSON is where they would
 * break it. Anything else is prose, and gets a plain textarea.
 *
 * Save and Approve hand over undefined when the text says what the gate
 * already had, so an editor opened and closed sends no rewrite.
 */
export function GateEditor({
  state,
  onSave,
  onCancel,
  onApprove,
  onDraftChange,
  large
}: {
  state: NodeExecutionState
  onSave: (edited: string | undefined) => void
  onCancel: () => void
  /** Offers Approve beside Save, for a host whose own Approve the editor replaces. */
  onApprove?: (edited: string | undefined) => void
  /** Told what the editor holds as it changes, for a host whose own Approve stays in view. */
  onDraftChange?: (draft: GateDraft) => void
  large?: boolean
}) {
  const original = state.editableText ?? ''
  const isTable = useMemo(() => {
    const list = toItemList(original)
    return !('error' in list) && isRecordList(list.items)
  }, [original])

  if (isTable) {
    return (
      <GateJsonEditor
        original={original}
        initial={state.editedText ?? original}
        onSave={onSave}
        onCancel={onCancel}
        onApprove={onApprove}
        onDraftChange={onDraftChange}
        large={large}
      />
    )
  }
  return (
    <GateTextEditor
      state={state}
      onSave={onSave}
      onCancel={onCancel}
      onApprove={onApprove}
      onDraftChange={onDraftChange}
      large={large}
    />
  )
}

function GateTextEditor({
  state,
  onSave,
  onCancel,
  onApprove,
  onDraftChange,
  large
}: Parameters<typeof GateEditor>[0]) {
  const original = state.editableText ?? ''
  const [text, setText] = useState(state.editedText ?? original)
  const size = large ? 'px-3.5 py-2 text-[12.5px]' : 'px-2 py-1 text-[11px]'
  const words = text.trim() ? text.trim().split(/\s+/).length : 0
  const edited = text.trim() === original.trim() ? undefined : text

  useReportDraft(onDraftChange, { edited, invalid: null }, [edited])

  return (
    <div className="flex flex-col gap-2">
      <textarea
        id={`gate-edit-${state.nodeId}`}
        aria-label="The text to approve"
        autoFocus
        rows={8}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation()
            onCancel()
          } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault()
            onSave(edited)
          }
        }}
        className="w-full px-2.5 py-2 bg-white/[0.03] border border-white/[0.2] rounded-md
                   text-[12.5px] leading-[1.5] text-gray-200 placeholder:text-gray-600
                   focus:outline-none resize-none"
      />
      <div className="flex items-center gap-2 text-[11px] leading-[1.45] text-ink-faint">
        <span>
          {words} {words === 1 ? 'word' : 'words'}
        </span>
        {text !== original && <span className="text-ink-secondary">Edited</span>}
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => setText(original)}
          disabled={text === original}
          className="hover:text-ink-secondary transition-colors disabled:opacity-40 disabled:pointer-events-none"
        >
          Revert
        </button>
      </div>
      <div className="flex items-center justify-end gap-1.5">
        <button
          type="button"
          onClick={onCancel}
          className={`${size} text-ink-faint hover:text-ink-secondary transition-colors`}
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={() => onSave(edited)}
          className={`flex items-center gap-1 ${size} ${GATE_NEUTRAL}`}
        >
          <Pencil size={large ? 13 : 11} strokeWidth={1.75} />
          Save
        </button>
        {onApprove && (
          <button
            type="button"
            onClick={() => onApprove(edited)}
            className={`flex items-center gap-1 ${size} ${GATE_APPROVE}`}
          >
            <Check size={large ? 13 : 11} strokeWidth={2.5} />
            Approve
          </button>
        )}
      </div>
    </div>
  )
}

interface Props {
  runId: string
  state: NodeExecutionState
  config?: ApprovalConfig
  nodes: WorkflowNode[]
  onOpenReview?: () => void
  /** Comments left on the review page, which Request changes carries back. */
  comments?: GateComment[]
}

/** Open review, Request changes, Reject and Approve, in the order a reviewer reaches for them. */
export function GateActions({ runId, state, config, nodes, onOpenReview, comments }: Props) {
  const [composing, setComposing] = useState<'changes' | 'reject' | 'edit' | null>(null)
  // Kept until the reviewer answers: approving sends it, sending the work back
  // carries it, rejecting drops it with the run.
  const [edited, setEdited] = useState<string | undefined>(undefined)

  if (composing === 'edit') {
    return (
      <GateEditor
        // The gate clears its own editedText when it opens, so reopening the editor
        // has to be handed the rewrite that has not been sent yet.
        state={{ ...state, ...(edited !== undefined && { editedText: edited }) }}
        onSave={(next) => {
          setEdited(next)
          setComposing(null)
        }}
        onCancel={() => setComposing(null)}
        // The row with Approve on it gives way to the editor, so the editor carries
        // one; it is shut while the draft cannot be sent, and says why.
        onApprove={(next) => void answerGate(runId, state.nodeId, 'approve', undefined, next)}
      />
    )
  }

  if (composing) {
    return (
      <GateComposer
        runId={runId}
        state={state}
        config={config}
        nodes={nodes}
        kind={composing}
        edited={edited}
        comments={comments}
        onDone={() => setComposing(null)}
      />
    )
  }

  return (
    <div className="flex items-center gap-1.5 flex-wrap">
      {state.editableText !== undefined && (
        <button
          type="button"
          onClick={() => setComposing('edit')}
          className={`flex items-center gap-1 px-2 py-1 text-[11px] ${GATE_NEUTRAL}`}
        >
          <Pencil size={11} strokeWidth={1.75} />
          {edited ? 'Edited' : 'Edit'}
        </button>
      )}
      {state.viewToken && onOpenReview && (
        <button
          type="button"
          onClick={onOpenReview}
          className={`flex items-center gap-1 px-2 py-1 text-[11px] ${GATE_NEUTRAL}`}
        >
          <Eye size={11} strokeWidth={1.75} />
          Open review
        </button>
      )}
      <span className="flex-1" />
      {canRequestChanges(config, state) && (
        <button
          type="button"
          onClick={() => setComposing('changes')}
          className={`flex items-center gap-1 px-2 py-1 text-[11px] ${GATE_NEUTRAL}`}
        >
          <MessageSquare size={11} strokeWidth={1.75} />
          {comments?.length ? `Request changes · ${comments.length}` : 'Request changes'}
        </button>
      )}
      <button
        type="button"
        onClick={() => setComposing('reject')}
        className={`flex items-center gap-1 px-2 py-1 text-[11px] ${GATE_REJECT}`}
      >
        <X size={11} strokeWidth={2.5} />
        Reject
      </button>
      <button
        type="button"
        onClick={() => void answerGate(runId, state.nodeId, 'approve', undefined, edited)}
        className={`flex items-center gap-1 px-2 py-1 text-[11px] ${GATE_APPROVE}`}
      >
        <Check size={11} strokeWidth={2.5} />
        Approve
      </button>
    </div>
  )
}
