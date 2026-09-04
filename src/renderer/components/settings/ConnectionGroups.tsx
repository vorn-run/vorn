import type { ReactNode } from 'react'
import { Plus, AlertTriangle } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import { ConnectionRow } from './ConnectionRow'
import type { RowActivity } from '../../lib/use-row-action'
import { groupConnections, type ConnectorListing } from '../../lib/connector-browse'
import type { ConnectorManifest, SourceConnection, WorkflowDefinition } from '../../../shared/types'

export interface ConnectorStatus {
  connectorId: string
  authed: boolean
  message?: string
}

/**
 * What you have, grouped by what kind of thing it is.
 *
 * Two connections to the same connector are one heading with two rows under
 * it. That is the only place a count belongs — beside the things it counts,
 * rather than on a catalog card trying to sell you a third.
 *
 * A connector that is not signed in says so here rather than in a banner at
 * the top of the page, which was far from the rows it was about. That warning
 * is a fact about the connector, not about any one connection; a connection's
 * own failure shows on its own row.
 */
export function ConnectionGroups({
  connections,
  listings,
  manifests,
  statuses,
  workflows,
  activity,
  backfillResult,
  onAdd,
  onRun,
  onBackfill,
  onDelete,
  onResetWorkflow,
  onOpenWorkflow,
  onRefresh
}: {
  connections: SourceConnection[]
  listings: ConnectorListing[]
  manifests: Record<string, ConnectorManifest | undefined>
  statuses: ConnectorStatus[]
  workflows: WorkflowDefinition[]
  activity: RowActivity
  backfillResult: Record<string, { imported: number; updated: number; error?: string }>
  onAdd: (listing: ConnectorListing) => void
  onRun: (workflowId: string) => void
  onBackfill: (connectionId: string) => void
  onDelete: (connectionId: string) => void
  onResetWorkflow: (connectionId: string, event: string) => void
  onOpenWorkflow: (workflowId: string) => void
  onRefresh: () => void
}) {
  const groups = groupConnections(connections, listings)

  return (
    <div className="space-y-5">
      {groups.map((group) => {
        const manifest = manifests[group.connectorId]
        const status = statuses.find((s) => s.connectorId === group.connectorId && !s.authed)

        return (
          <div key={group.connectorId}>
            <div className="flex items-center gap-3 mb-2">
              <span className="w-8 h-8 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
                <ConnectorIcon
                  connectorId={group.connectorId}
                  icon={group.icon}
                  size={17}
                  className="text-gray-200"
                />
              </span>
              <div className="min-w-0">
                <div className="text-[13.5px] text-gray-200 font-medium">{group.name}</div>
                <div className="text-[11px] text-gray-600">
                  {count(group.connections.length, 'connection')}
                  {group.version && ` · v${group.version}`}
                </div>
              </div>
              {/* Absent for a package installed by name: there is no catalog
                  entry to open a form against. */}
              {group.listing && (
                <button
                  onClick={() => onAdd(group.listing!)}
                  className="ml-auto text-[11px] text-gray-500 hover:text-gray-200 px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
                >
                  <Plus size={11} /> Add another
                </button>
              )}
            </div>

            {status && (
              <div className="flex items-start gap-2 mb-2 px-3 py-2 border border-amber-500/30 bg-amber-500/[0.04] rounded-sm">
                <AlertTriangle size={13} className="text-amber-400 mt-0.5 shrink-0" />
                <div className="text-[11.5px] text-amber-200/90 leading-snug whitespace-pre-line">
                  {renderMessageWithCode(status.message || 'Not signed in.')}
                </div>
              </div>
            )}

            <div className="space-y-2">
              {group.connections.map((conn) => (
                <ConnectionRow
                  key={conn.id}
                  conn={conn}
                  manifest={manifest}
                  seededWorkflows={seededFor(workflows, conn)}
                  missingEvents={missingFor(workflows, conn, manifest)}
                  activity={activity}
                  backfillResult={backfillResult}
                  onRun={onRun}
                  onBackfill={onBackfill}
                  onDelete={onDelete}
                  onResetWorkflow={onResetWorkflow}
                  onOpenWorkflow={onOpenWorkflow}
                  onRefresh={onRefresh}
                />
              ))}
            </div>
          </div>
        )
      })}
    </div>
  )
}

/** The workflows seeded for a connection, which poll it on a schedule. */
function seededFor(workflows: WorkflowDefinition[], conn: SourceConnection): WorkflowDefinition[] {
  const prefix = `connector:${conn.id}:`
  return workflows.filter((w) => w.id.startsWith(prefix))
}

/**
 * Events the connector expects to poll but has no workflow for.
 *
 * Deleting the seeded workflow silently stops the polling, so the row says so
 * and offers to put it back.
 */
function missingFor(
  workflows: WorkflowDefinition[],
  conn: SourceConnection,
  manifest: ConnectorManifest | undefined
): Array<{ name: string; event: string }> {
  const seeded = seededFor(workflows, conn)
  return (manifest?.defaultWorkflows ?? []).filter(
    (e) => !seeded.some((w) => w.id === `connector:${conn.id}:${e.event}`)
  )
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}

/**
 * A message with `backticked` commands set in monospace.
 *
 * Sign-in messages are mostly a command to run, and a command that does not
 * look like one is easy to misread as prose.
 */
function renderMessageWithCode(text: string): ReactNode {
  const parts = text.split(/(`[^`]+`)/g)
  return parts.map((part, i) => {
    if (part.startsWith('`') && part.endsWith('`') && part.length >= 2) {
      return (
        <code
          key={i}
          className="px-1 py-[1px] bg-black/30 border border-white/[0.08] rounded-sm text-amber-100 font-mono text-[11px]"
        >
          {part.slice(1, -1)}
        </code>
      )
    }
    return <span key={i}>{part}</span>
  })
}
