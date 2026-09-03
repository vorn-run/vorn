import { useState } from 'react'
import { Play, Trash2, AlertCircle, Workflow, Import, Activity } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { ConnectorIcon } from '../ConnectorIcon'
import { connectionIcon } from '../../lib/connection-icon'
import { McpToolsPanel } from './McpToolsPanel'
import { humanCron } from '../../lib/cron-text'
import type { ConnectorManifest, SourceConnection, WorkflowDefinition } from '../../../shared/types'

/**
 * One configured connection: what it polls, when it last ran, and what can be
 * done to it.
 *
 * Lifted out of ConnectorSettings unchanged when connections were separated
 * from the catalog. A connection is the thing people come to this page to
 * check on, so everything it already showed — the workflows polling it and
 * their schedules, backfill and its result, sync errors, the filters it was
 * configured with — is still here.
 */
export function ConnectionRow({
  conn,
  manifest,
  seededWorkflows,
  missingEvents,
  runningId,
  backfillingId,
  backfillResult,
  onRun,
  onBackfill,
  onDelete,
  onResetWorkflow,
  onOpenWorkflow,
  onRefresh
}: {
  conn: SourceConnection
  manifest?: ConnectorManifest
  seededWorkflows: WorkflowDefinition[]
  missingEvents: Array<{ name: string; event: string }>
  runningId: string | null
  backfillingId: string | null
  backfillResult: Record<string, { imported: number; updated: number; error?: string }>
  onRun: (workflowId: string) => void
  onBackfill: (connectionId: string) => void
  onDelete: (connectionId: string) => void
  onResetWorkflow: (connectionId: string, event: string) => void
  onOpenWorkflow: (workflowId: string) => void
  onRefresh: () => void
}) {
  // Made by the app for a connector that asks for nothing: it goes when the
  // connector does, so deleting the row on its own would only bring it back.
  const implicit = conn.filters.implicit === true
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean | null; message?: string } | null>(
    null
  )

  const runTest = async () => {
    setTesting(true)
    setTestResult(null)
    try {
      setTestResult(await window.api.preflightConnection(conn.id))
    } catch (err) {
      setTestResult({ ok: false, message: err instanceof Error ? err.message : String(err) })
    } finally {
      setTesting(false)
    }
  }

  return (
    <div className="px-4 py-2 bg-white/[0.03] border border-white/[0.06] rounded-sm">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3 min-w-0">
          <span className="w-6 h-6 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
            <ConnectorIcon
              connectorId={conn.connectorId}
              icon={connectionIcon(conn)}
              size={14}
              className="text-gray-200"
            />
          </span>
          <span className="text-sm text-gray-200 font-medium truncate">{conn.name}</span>
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
          <Tooltip label="Import existing items matching this connection's filters. Bypasses the cron cursor.">
            <button
              onClick={() => onBackfill(conn.id)}
              disabled={backfillingId === conn.id}
              className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
            >
              <Import size={13} className={backfillingId === conn.id ? 'animate-pulse' : ''} />
            </button>
          </Tooltip>
          {conn.connectorId === 'http' && (
            <Tooltip label="Send a request through this profile now and report the status">
              <button
                onClick={runTest}
                disabled={testing}
                className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
              >
                <Activity size={13} className={testing ? 'animate-pulse' : ''} />
              </button>
            </Tooltip>
          )}
          <Tooltip
            label={
              implicit
                ? 'This connection came with its connector. Remove the pack instead.'
                : 'Remove this connection (seeded workflows are also deleted)'
            }
          >
            <button
              onClick={() => onDelete(conn.id)}
              disabled={implicit}
              className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-40 disabled:hover:text-gray-500"
            >
              <Trash2 size={13} />
            </button>
          </Tooltip>
        </div>
      </div>

      {testResult && (
        <div className={`mt-1 text-[11px] ${testResult.ok ? 'text-green-400' : 'text-red-400'}`}>
          {testResult.message || (testResult.ok ? 'Reachable' : 'Failed')}
        </div>
      )}

      {/* Polled-by-workflow rows — make the mechanism visible */}
      <div className="mt-1.5 space-y-1">
        {seededWorkflows.map((wf) => {
          const trigger = wf.nodes.find((n) => n.type === 'trigger')
          const cron =
            trigger?.config && 'cron' in trigger.config
              ? (trigger.config as { cron: string }).cron
              : ''
          return (
            <div
              key={wf.id}
              className="flex items-center justify-between text-[11px] text-gray-500"
            >
              <Tooltip label="Open this workflow in the editor to customize the schedule, filters, or add steps">
                <button
                  onClick={() => onOpenWorkflow(wf.id)}
                  className="flex items-center gap-1.5 text-gray-400 hover:text-white transition-colors group"
                >
                  <Workflow size={12} className="text-gray-500" strokeWidth={1.75} />
                  <span className="group-hover:underline underline-offset-2 decoration-white/30">
                    {wf.name}
                  </span>
                  <span className="text-gray-600">· {humanCron(cron)}</span>
                </button>
              </Tooltip>
              <Tooltip label="Poll the connector now instead of waiting for the next cron tick">
                <button
                  onClick={() => onRun(wf.id)}
                  disabled={runningId === wf.id}
                  className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
                >
                  <Play size={11} className={runningId === wf.id ? 'animate-pulse' : ''} />
                </button>
              </Tooltip>
            </div>
          )
        })}

        {missingEvents.map((e) => (
          <div
            key={e.event}
            className="flex items-center justify-between text-[11px] text-gray-500"
          >
            <span className="text-gray-600 italic">
              No workflow for {e.name} — polling disabled
            </span>
            <Tooltip label="Re-seed the default workflow for this event (same as when you first connected)">
              <button
                onClick={() => onResetWorkflow(conn.id, e.event)}
                className="text-[10px] text-gray-400 hover:text-gray-200 px-2 py-0.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors"
              >
                Reset default workflow
              </button>
            </Tooltip>
          </div>
        ))}
      </div>

      <div className="mt-1 flex items-center gap-2 text-[11px]">
        {conn.lastSyncAt && (
          <span className="text-gray-600">
            Last synced {new Date(conn.lastSyncAt).toLocaleString()}
          </span>
        )}
        {conn.lastSyncError && (
          <span className="text-red-400 flex items-center gap-1">
            <AlertCircle size={10} /> {conn.lastSyncError}
          </span>
        )}
        {backfillResult[conn.id] && !backfillResult[conn.id].error && (
          <span className="text-green-400">
            +{backfillResult[conn.id].imported} imported
            {backfillResult[conn.id].updated > 0 && `, ${backfillResult[conn.id].updated} updated`}
          </span>
        )}
        {backfillResult[conn.id]?.error && (
          <span className="text-red-400 flex items-center gap-1">
            <AlertCircle size={10} /> {backfillResult[conn.id].error}
          </span>
        )}
      </div>

      {Object.keys(conn.filters).length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {Object.entries(conn.filters)
            // `discoveredTools` on MCP rows is rendered in the
            // Tools panel below, not as a tag. Skip it here.
            .filter(([k]) => k !== 'discoveredTools')
            .map(([k, v]) => {
              const isSecret = (manifest?.auth ?? []).some(
                (f) => f.key === k && f.type === 'password'
              )
              const display = isSecret
                ? '••••••'
                : typeof v === 'string' && v.length > 60
                  ? v.slice(0, 57) + '…'
                  : String(v)
              return (
                <span
                  key={k}
                  className="text-[10px] px-1.5 py-0.5 bg-white/[0.04] rounded-sm text-gray-400"
                >
                  {k}: {display}
                </span>
              )
            })}
        </div>
      )}

      {conn.connectorId === 'mcp' && <McpToolsPanel connection={conn} onRefresh={onRefresh} />}
    </div>
  )
}
