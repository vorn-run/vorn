import { useEffect, useState } from 'react'
import { Lock, X } from 'lucide-react'
import type { ApprovalConfig, NodeExecutionState, WorkflowNode } from '../../../shared/types'
import { NODE_TYPE_ICON } from '../workflow-editor/node-visuals'
import { gateViewUrl } from '../../lib/gate-view-url'
import { GateActions } from './GateActions'
import { roundLabel } from '../../lib/gate-round'

interface Props {
  runId: string
  workflowName?: string
  state: NodeExecutionState
  node: WorkflowNode
  nodes: WorkflowNode[]
  onClose: () => void
}

/** The step whose output the page is, so the footer can say what built it. */
function builtBy(view: string | undefined, nodes: WorkflowNode[]): string | undefined {
  const slug = view?.match(/\{\{\s*steps\.([\w-]+)\./)?.[1]
  return slug ? nodes.find((n) => n.slug === slug)?.label : undefined
}

export function GateReviewModal({ runId, workflowName, state, node, nodes, onClose }: Props) {
  const config = node.config as ApprovalConfig
  const token = state.viewToken
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    if (!token) return
    let live = true
    gateViewUrl(runId, state.nodeId, token).then(
      (next) => live && setUrl(next),
      () => live && setFailed(true)
    )
    return () => {
      live = false
    }
  }, [runId, state.nodeId, token])

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const Icon = NODE_TYPE_ICON[node.type]
  const producer = builtBy(config.view, nodes)
  const meta = [workflowName, `run ${runId.slice(0, 8)}`, roundLabel(config, state)]
    .filter(Boolean)
    .join(' · ')

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Review: ${node.label}`}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    >
      <div className="w-[90vw] max-w-[960px] h-[84vh] bg-surface-panel border border-white/[0.1] rounded-xl flex flex-col overflow-hidden">
        <div className="flex items-center gap-2.5 px-4 h-11 border-b border-white/[0.08] shrink-0">
          {Icon && <Icon size={13} strokeWidth={1.5} className="text-ink-faint shrink-0" />}
          <span className="text-[13px] font-medium text-ink shrink-0">{node.label}</span>
          <span className="font-mono text-[11.5px] text-ink-faint truncate">{meta}</span>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close review"
            className="ml-auto p-1 rounded-md text-ink-faint hover:text-ink transition-colors"
          >
            <X size={14} />
          </button>
        </div>

        {url ? (
          <iframe
            title={`Review page for ${node.label}`}
            src={url}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            className="flex-1 w-full border-0 bg-white"
          />
        ) : (
          <div className="flex-1 flex items-center justify-center text-[12px] text-ink-faint">
            {failed || !token ? 'The review page could not be opened.' : 'Opening the review page…'}
          </div>
        )}

        <div className="flex items-end gap-4 px-4 py-2.5 border-t border-white/[0.08] shrink-0">
          <div className="flex-1 min-w-0 flex items-center gap-1.5 pb-1 text-[11px] text-ink-faint">
            <Lock size={11} strokeWidth={2} className="shrink-0" />
            <span className="truncate">
              {producer && (
                <>
                  Built by <span className="text-ink-secondary">{producer}</span>.{' '}
                </>
              )}
              It runs sandboxed: nothing loads from the network and it can&apos;t reach Vorn.
            </span>
          </div>
          <div className="shrink-0 w-[min(440px,55%)]">
            <GateActions runId={runId} state={state} config={config} nodes={nodes} />
          </div>
        </div>
      </div>
    </div>
  )
}
