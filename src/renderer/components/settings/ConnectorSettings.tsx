import { useState, useEffect, useCallback, type ReactNode } from 'react'
import { useAppStore } from '../../stores'
import { SettingsPageHeader } from './SettingsPageHeader'
import { ConnectorIcon } from '../ConnectorIcon'
import {
  Plus,
  Play,
  Trash2,
  Check,
  AlertCircle,
  AlertTriangle,
  Workflow,
  Import,
  ChevronDown,
  ChevronRight,
  RefreshCw,
  Wrench
} from 'lucide-react'
import { Tooltip } from '../Tooltip'
import type {
  SourceConnection,
  ConnectorManifest,
  ConnectorConfigField,
  TaskStatus,
  WorkflowDefinition
} from '../../../shared/types'
import { schemaProperties, schemaTypeHint } from '../../../shared/json-schema-utils'
import { SdkConnectorForm } from './SdkConnectorForm'

/** Connector id the generic MCP stdio connection registers under. */
const MCP_CONNECTOR_ID = 'mcp'

interface ConnectorInfo {
  id: string
  name: string
  icon: string
  capabilities: string[]
  manifest: ConnectorManifest
}

interface ConnectorStatus {
  connectorId: string
  authed: boolean
  message?: string
}

function humanCron(cron: string): string {
  const m = cron.match(/^\*\/(\d+) \* \* \* \*$/)
  if (m) return `every ${m[1]} minute${m[1] === '1' ? '' : 's'}`
  if (cron === '* * * * *') return 'every minute'
  return cron
}

/** Render text with `backtick` spans styled as inline code. */
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

export function ConnectorSettings() {
  const workflows = useAppStore((s) => s.config?.workflows ?? [])
  const openWorkflowEditor = (id: string) => {
    const store = useAppStore.getState()
    // Close settings, switch to workflows view, select the workflow, and
    // open the editor. Settings is a modal overlay — if we only set the
    // editor state, the editor renders behind settings and looks dead.
    store.setSettingsOpen(false)
    store.setMainViewMode('workflows')
    store.setEditingWorkflowId(id)
    store.setWorkflowEditorOpen(true)
  }
  const [connectors, setConnectors] = useState<ConnectorInfo[]>([])
  const [connections, setConnections] = useState<SourceConnection[]>([])
  const [statuses, setStatuses] = useState<ConnectorStatus[]>([])
  const [adding, setAdding] = useState<string | null>(null)
  const [runningId, setRunningId] = useState<string | null>(null)
  const [backfillingId, setBackfillingId] = useState<string | null>(null)
  const [backfillResult, setBackfillResult] = useState<
    Record<string, { imported: number; updated: number; error?: string }>
  >({})

  const load = useCallback(async () => {
    const [c, conns, st] = await Promise.all([
      window.api.listConnectors(),
      window.api.listConnections(),
      window.api.getConnectorStatus()
    ])
    setConnectors(c)
    setConnections(conns)
    setStatuses(st)
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: loads connectors from the main process on mount
    void load()
  }, [load])

  const findSeededWorkflows = (conn: SourceConnection): WorkflowDefinition[] => {
    const prefix = `connector:${conn.id}:`
    return workflows.filter((w) => w.id.startsWith(prefix))
  }

  const handleRun = async (workflowId: string) => {
    setRunningId(workflowId)
    try {
      await window.api.runWorkflowManual(workflowId)
    } finally {
      setTimeout(() => setRunningId(null), 800)
      load()
    }
  }

  const handleReset = async (connectionId: string, event: string) => {
    await window.api.seedConnectorWorkflow(connectionId, event)
    load()
  }

  const handleBackfill = async (connectionId: string) => {
    setBackfillingId(connectionId)
    setBackfillResult((prev) => {
      const { [connectionId]: _removed, ...rest } = prev
      return rest
    })
    try {
      const result = await window.api.backfillConnection(connectionId)
      setBackfillResult((prev) => ({ ...prev, [connectionId]: result }))
    } finally {
      setBackfillingId(null)
      load()
    }
  }

  const handleDelete = async (connectionId: string) => {
    await window.api.deleteConnection(connectionId)
    load()
  }

  return (
    <div>
      <SettingsPageHeader
        title="Connectors"
        description="Connect external task sources like GitHub. Each connection seeds a visible, editable workflow that polls on cron."
      />

      {/* Auth status banners (non-authed connectors only) */}
      {statuses
        .filter((s) => !s.authed)
        .map((s) => (
          <div
            key={s.connectorId}
            className="mb-4 flex items-start gap-2 px-3 py-2 border border-amber-500/30 bg-amber-500/[0.04] rounded-sm"
          >
            <AlertTriangle size={14} className="text-amber-400 mt-0.5 shrink-0" />
            <div className="text-[12px] text-amber-200 leading-snug">
              <div className="font-medium">
                {connectors.find((c) => c.id === s.connectorId)?.name || s.connectorId} not signed
                in
              </div>
              <div className="text-amber-300/70 mt-0.5 whitespace-pre-line">
                {renderMessageWithCode(
                  s.message || 'Run `gh auth login` in your terminal to sign in.'
                )}
              </div>
            </div>
          </div>
        ))}

      {/* Available connectors */}
      <div className="mb-6">
        <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-3">
          Available Connectors
        </h3>
        <div className="space-y-1">
          {connectors.map((c) => {
            const existingConns = connections.filter((conn) => conn.connectorId === c.id)
            return (
              <div
                key={c.id}
                className="flex items-center justify-between px-4 py-2.5 bg-white/[0.03] border border-white/[0.06] rounded-sm"
              >
                <div className="flex items-center gap-3">
                  <span className="w-7 h-7 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
                    <ConnectorIcon connectorId={c.id} size={16} className="text-gray-200" />
                  </span>
                  <div>
                    <span className="text-sm text-gray-200 font-medium">{c.name}</span>
                    <span className="text-xs text-gray-500 ml-2">{c.capabilities.join(' · ')}</span>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {existingConns.length > 0 && (
                    <span className="text-xs text-green-500 flex items-center gap-1">
                      <Check size={12} /> {existingConns.length} connected
                    </span>
                  )}
                  <Tooltip label={`Add a ${c.name} connection`}>
                    <button
                      onClick={() => setAdding(c.id)}
                      className="text-xs text-gray-400 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
                    >
                      <Plus size={12} /> Add
                    </button>
                  </Tooltip>
                </div>
              </div>
            )
          })}
          {connectors.length === 0 && (
            <p className="text-sm text-gray-500">No connectors available.</p>
          )}
        </div>
      </div>

      {adding && (
        <AddConnectionForm
          connector={connectors.find((c) => c.id === adding)!}
          onDone={() => {
            setAdding(null)
            load()
          }}
          onCancel={() => setAdding(null)}
        />
      )}

      {connections.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-3">
            Connected
          </h3>
          <div className="space-y-2">
            {connections.map((conn) => {
              const connector = connectors.find((c) => c.id === conn.connectorId)
              const seededWorkflows = findSeededWorkflows(conn)
              const expectedEvents = connector?.manifest.defaultWorkflows ?? []
              const missingEvents = expectedEvents.filter(
                (e) => !seededWorkflows.some((w) => w.id === `connector:${conn.id}:${e.event}`)
              )

              return (
                <div
                  key={conn.id}
                  className="px-4 py-2 bg-white/[0.03] border border-white/[0.06] rounded-sm"
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3 min-w-0">
                      <span className="w-6 h-6 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
                        <ConnectorIcon
                          connectorId={conn.connectorId}
                          size={14}
                          className="text-gray-200"
                        />
                      </span>
                      <span className="text-sm text-gray-200 font-medium truncate">
                        {conn.name}
                      </span>
                    </div>
                    <div className="flex items-center gap-0.5 shrink-0">
                      <Tooltip label="Import existing items matching this connection's filters. Bypasses the cron cursor.">
                        <button
                          onClick={() => handleBackfill(conn.id)}
                          disabled={backfillingId === conn.id}
                          className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
                        >
                          <Import
                            size={13}
                            className={backfillingId === conn.id ? 'animate-pulse' : ''}
                          />
                        </button>
                      </Tooltip>
                      <Tooltip label="Remove this connection (seeded workflows are also deleted)">
                        <button
                          onClick={() => handleDelete(conn.id)}
                          className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors"
                        >
                          <Trash2 size={13} />
                        </button>
                      </Tooltip>
                    </div>
                  </div>

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
                              onClick={() => openWorkflowEditor(wf.id)}
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
                              onClick={() => handleRun(wf.id)}
                              disabled={runningId === wf.id}
                              className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
                            >
                              <Play
                                size={11}
                                className={runningId === wf.id ? 'animate-pulse' : ''}
                              />
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
                            onClick={() => handleReset(conn.id, e.event)}
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
                        {backfillResult[conn.id].updated > 0 &&
                          `, ${backfillResult[conn.id].updated} updated`}
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
                          const isSecret = (connector?.manifest.auth ?? []).some(
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

                  {conn.connectorId === 'mcp' && (
                    <McpToolsPanel connection={conn} onRefresh={load} />
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

function AddConnectionForm({
  connector,
  onDone,
  onCancel
}: {
  connector: ConnectorInfo
  onDone: () => void
  onCancel: () => void
}) {
  const projects = useAppStore((s) => s.config?.projects || [])
  const manifest = connector.manifest
  const usesRepoDetect = connector.id === 'github'
  // An MCP connection is either a connector package that describes itself or
  // a raw server the user wires up by hand. The first covers most cases, so
  // it leads.
  const isMcp = connector.id === MCP_CONNECTOR_ID
  const [fromPackage, setFromPackage] = useState(isMcp)

  const [selectedProject, setSelectedProject] = useState(projects[0]?.name || '')
  const [detectedRepo, setDetectedRepo] = useState<{
    owner: string
    repo: string
  } | null>(null)
  const [detecting, setDetecting] = useState(false)
  const [auth, setAuth] = useState<Record<string, string>>({})
  const [filters, setFilters] = useState<Record<string, string>>({})
  const [manualRepo, setManualRepo] = useState<{ owner: string; repo: string }>({
    owner: '',
    repo: ''
  })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!usesRepoDetect) return
    const project = projects.find((p) => p.name === selectedProject)
    if (!project) return
    /* eslint-disable react-hooks/set-state-in-effect -- intentional: marks detection in flight before querying the main process */
    setDetecting(true)
    setDetectedRepo(null)
    /* eslint-enable react-hooks/set-state-in-effect */
    window.api.detectRepo(project.path).then((result) => {
      setDetectedRepo(result)
      setDetecting(false)
    })
  }, [selectedProject, projects, usesRepoDetect])

  const statusMapping: Record<string, TaskStatus> = {}
  for (const opt of manifest.statusMapping || []) {
    statusMapping[opt.upstream] = opt.suggestedLocal
  }

  const missingAuth = (manifest.auth ?? []).some((f) => f.required && !auth[f.key]?.trim())
  const manualRepoValid = manualRepo.owner.trim().length > 0 && manualRepo.repo.trim().length > 0
  const canSave = usesRepoDetect ? !!detectedRepo || manualRepoValid : !missingAuth

  const handleSave = async () => {
    setError(null)
    setSaving(true)
    try {
      // Encrypt any password-typed auth fields via Electron's safeStorage
      // BEFORE they touch the DB. Plaintext never leaves this call.
      const encryptedAuth: Record<string, string> = {}
      for (const field of manifest.auth ?? []) {
        const v = auth[field.key]
        if (!v) continue
        if (field.type === 'password') {
          try {
            encryptedAuth[field.key] = await window.api.encryptString(v)
          } catch (err) {
            throw new Error(
              `Could not encrypt ${field.label}: ${err instanceof Error ? err.message : String(err)}. ` +
                `OS keychain access may be unavailable.`,
              { cause: err }
            )
          }
        } else {
          encryptedAuth[field.key] = v
        }
      }

      const connectionFilters: Record<string, unknown> = { ...encryptedAuth, ...filters }
      let name: string
      if (usesRepoDetect) {
        const owner = detectedRepo?.owner ?? manualRepo.owner.trim()
        const repo = detectedRepo?.repo ?? manualRepo.repo.trim()
        connectionFilters.owner = owner
        connectionFilters.repo = repo
        name = `${owner}/${repo}`
      } else {
        name =
          (filters.teamKey && `${connector.name}: ${filters.teamKey}`) ||
          `${connector.name}${selectedProject ? ` · ${selectedProject}` : ''}`
      }
      await window.api.createConnection({
        connectorId: connector.id,
        name,
        filters: connectionFilters,
        syncIntervalMinutes: 5,
        statusMapping,
        executionProject: selectedProject
      })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="mb-6 px-4 py-3 bg-white/[0.03] border border-white/[0.08] rounded-sm">
      <div className="flex items-center gap-2 mb-3">
        <ConnectorIcon connectorId={connector.id} size={14} className="text-gray-400" />
        <h3 className="text-sm font-medium text-gray-200">Connect {connector.name}</h3>
      </div>

      {isMcp && (
        <div className="flex gap-1 mb-3">
          {[
            { key: true, label: 'From a package' },
            { key: false, label: 'Manual setup' }
          ].map((tab) => (
            <button
              key={String(tab.key)}
              onClick={() => setFromPackage(tab.key)}
              className={`px-3 py-1 text-xs rounded-sm transition-colors ${
                fromPackage === tab.key
                  ? 'bg-white/[0.1] text-gray-200'
                  : 'text-gray-500 hover:text-gray-300'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      )}

      {isMcp && fromPackage ? (
        <SdkConnectorForm onDone={onDone} onCancel={onCancel} />
      ) : (
        <div className="space-y-3">
          <div>
            <label className="block text-xs text-gray-500 mb-1">Project</label>
            <select
              value={selectedProject}
              onChange={(e) => setSelectedProject(e.target.value)}
              className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
            >
              {projects.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>

          {usesRepoDetect && (
            <>
              <div className="text-xs">
                {detecting && <span className="text-gray-500">Detecting repository...</span>}
                {detectedRepo && (
                  <span className="text-green-400 flex items-center gap-1">
                    <Check size={12} /> Detected: {detectedRepo.owner}/{detectedRepo.repo}
                  </span>
                )}
                {!detecting && !detectedRepo && selectedProject && (
                  <span className="text-amber-400">
                    No GitHub repo detected. Enter the repo manually below.
                  </span>
                )}
              </div>

              {/* Manual fallback — only visible when auto-detect failed. Covers
                GH Enterprise, non-standard remotes, and detached repos. */}
              {!detecting && !detectedRepo && (
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Owner</label>
                    <input
                      type="text"
                      value={manualRepo.owner}
                      onChange={(e) =>
                        setManualRepo((prev) => ({ ...prev, owner: e.target.value }))
                      }
                      placeholder="e.g. octocat"
                      className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Repository</label>
                    <input
                      type="text"
                      value={manualRepo.repo}
                      onChange={(e) => setManualRepo((prev) => ({ ...prev, repo: e.target.value }))}
                      placeholder="e.g. hello-world"
                      className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
                    />
                  </div>
                </div>
              )}
            </>
          )}

          {(manifest.auth ?? []).map((field) => (
            <DynamicField
              key={field.key}
              field={field}
              value={auth[field.key] || ''}
              onChange={(v) => setAuth((prev) => ({ ...prev, [field.key]: v }))}
            />
          ))}

          {(manifest.taskFilters || []).map((field) => (
            <DynamicField
              key={field.key}
              field={field}
              value={filters[field.key] || ''}
              onChange={(v) => setFilters((prev) => ({ ...prev, [field.key]: v }))}
            />
          ))}

          {error && (
            <div className="text-[11px] text-red-400 flex items-start gap-1">
              <AlertCircle size={12} className="mt-0.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          <div className="flex gap-2 pt-1">
            <button
              onClick={handleSave}
              disabled={saving || !canSave}
              className="px-4 py-1.5 text-sm bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50"
            >
              {saving ? 'Connecting...' : 'Connect'}
            </button>
            <button
              onClick={onCancel}
              className="px-4 py-1.5 text-sm text-gray-500 hover:text-gray-300 transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

interface McpTool {
  name: string
  description?: string
  inputSchema?: Record<string, unknown>
}

function McpToolsPanel({
  connection,
  onRefresh
}: {
  connection: SourceConnection
  onRefresh: () => void | Promise<void>
}) {
  const [expanded, setExpanded] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [selectedTool, setSelectedTool] = useState<McpTool | null>(null)

  const tools = Array.isArray(connection.filters.discoveredTools)
    ? (connection.filters.discoveredTools as McpTool[])
    : []

  const handleRefresh = async () => {
    setRefreshing(true)
    setRefreshError(null)
    try {
      const result = await window.api.refreshMcpTools(connection.id)
      if (!result.ok) setRefreshError(result.error ?? 'Refresh failed')
      // Pull fresh connections into the parent so `filters.discoveredTools`
      // on this row reflects whatever the server just wrote.
      await onRefresh()
    } catch (err) {
      setRefreshError(err instanceof Error ? err.message : String(err))
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <div className="mt-2 border-t border-white/[0.06] pt-2">
      <div className="flex items-center justify-between">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-white transition-colors"
        >
          {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          <Wrench size={11} />
          <span>
            {tools.length > 0
              ? `${tools.length} tool${tools.length === 1 ? '' : 's'}`
              : 'No tools discovered yet'}
          </span>
        </button>
        <Tooltip label="Re-run tools/list on the MCP server">
          <button
            onClick={handleRefresh}
            disabled={refreshing}
            className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
          >
            <RefreshCw size={11} className={refreshing ? 'animate-spin' : ''} />
          </button>
        </Tooltip>
      </div>

      {refreshError && (
        <div className="mt-1 text-[11px] text-red-400 flex items-start gap-1">
          <AlertCircle size={10} className="mt-0.5 shrink-0" />
          <span>{refreshError}</span>
        </div>
      )}

      {expanded && (
        <div className="mt-2 space-y-1">
          {tools.map((tool) => (
            <div
              key={tool.name}
              className="flex items-start justify-between gap-2 px-2 py-1 bg-white/[0.02] rounded-sm"
            >
              <div className="min-w-0">
                <div className="text-[12px] text-gray-200 font-mono truncate">{tool.name}</div>
                {tool.description && (
                  <div className="text-[10px] text-gray-500 leading-snug">{tool.description}</div>
                )}
              </div>
              <button
                onClick={() => setSelectedTool(tool)}
                className="text-[10px] text-gray-400 hover:text-white px-2 py-0.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors shrink-0"
              >
                Invoke
              </button>
            </div>
          ))}
          {tools.length === 0 && !refreshError && (
            <div className="text-[11px] text-gray-500 italic">
              Click refresh to run tools/list, or wait for initial discovery to complete.
            </div>
          )}
        </div>
      )}

      {selectedTool && (
        // Key by tool name so switching to a different tool remounts the
        // dialog with a fresh args stub + cleared result, instead of leaving
        // the previous tool's state behind.
        <InvokeToolDialog
          key={selectedTool.name}
          connectionId={connection.id}
          tool={selectedTool}
          onClose={() => setSelectedTool(null)}
        />
      )}
    </div>
  )
}

/**
 * Build a skeleton args object from an MCP tool's JSON Schema so the invoke
 * textarea isn't just `{}`. Covers the common cases (string/number/boolean/
 * array/object); unknowns fall back to null.
 */
function buildArgsStub(schema: Record<string, unknown> | undefined): string {
  const props = schemaProperties(schema)
  if (Object.keys(props).length === 0) return '{}'
  const stub: Record<string, unknown> = {}
  for (const [key, raw] of Object.entries(props)) {
    const def = (raw as { default?: unknown }).default
    if (def !== undefined) {
      stub[key] = def
      continue
    }
    switch (schemaTypeHint(raw)) {
      case 'string':
        stub[key] = ''
        break
      case 'number':
      case 'integer':
        stub[key] = 0
        break
      case 'boolean':
        stub[key] = false
        break
      case 'array':
        stub[key] = []
        break
      case 'object':
        stub[key] = {}
        break
      default:
        stub[key] = null
    }
  }
  return JSON.stringify(stub, null, 2)
}

function InvokeToolDialog({
  connectionId,
  tool,
  onClose
}: {
  connectionId: string
  tool: McpTool
  onClose: () => void
}) {
  const [argsText, setArgsText] = useState(() => buildArgsStub(tool.inputSchema))
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<{
    success: boolean
    output?: Record<string, unknown>
    error?: string
  } | null>(null)

  const handleRun = async () => {
    let args: Record<string, unknown>
    try {
      args = argsText.trim() === '' ? {} : JSON.parse(argsText)
    } catch (err) {
      setResult({
        success: false,
        error: `Invalid JSON: ${err instanceof Error ? err.message : String(err)}`
      })
      return
    }
    setRunning(true)
    try {
      const res = await window.api.executeConnectorAction({
        connectionId,
        action: tool.name,
        args
      })
      setResult(res)
    } finally {
      setRunning(false)
    }
  }

  return (
    <div className="mt-2 px-2 py-2 bg-black/30 border border-white/[0.08] rounded-sm">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[11px] font-mono text-gray-200">{tool.name}</span>
        <button
          onClick={onClose}
          className="text-[10px] text-gray-500 hover:text-gray-300 transition-colors"
        >
          Close
        </button>
      </div>
      {tool.inputSchema && (
        <details className="mb-2">
          <summary className="text-[10px] text-gray-500 cursor-pointer">inputSchema</summary>
          <pre className="mt-1 text-[10px] text-gray-400 whitespace-pre-wrap break-words max-h-32 overflow-auto">
            {JSON.stringify(tool.inputSchema, null, 2)}
          </pre>
        </details>
      )}
      <label className="block text-[10px] text-gray-500 mb-1">Arguments (JSON)</label>
      <textarea
        value={argsText}
        onChange={(e) => setArgsText(e.target.value)}
        rows={4}
        className="w-full px-2 py-1 bg-white/[0.05] border border-white/[0.1] rounded-sm text-[11px] font-mono text-gray-200 focus:border-white/[0.2] outline-none"
      />
      <div className="mt-2 flex gap-2">
        <button
          onClick={handleRun}
          disabled={running}
          className="px-3 py-1 text-[11px] bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50"
        >
          {running ? 'Running…' : 'Run'}
        </button>
      </div>
      {result && (
        <div className="mt-2">
          <div
            className={
              result.success
                ? 'text-[11px] text-green-400'
                : 'text-[11px] text-red-400 flex items-start gap-1'
            }
          >
            {result.success ? (
              <span>Success</span>
            ) : (
              <>
                <AlertCircle size={10} className="mt-0.5 shrink-0" />
                <span>{result.error}</span>
              </>
            )}
          </div>
          {result.output && (
            <pre className="mt-1 text-[10px] text-gray-400 whitespace-pre-wrap break-words max-h-48 overflow-auto bg-black/30 p-2 rounded-sm">
              {JSON.stringify(result.output, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}

function DynamicField({
  field,
  value,
  onChange
}: {
  field: ConnectorConfigField
  value: string
  onChange: (v: string) => void
}) {
  const isSecret = field.type === 'password'
  return (
    <div>
      <label className="block text-xs text-gray-500 mb-1 flex items-center gap-1.5">
        <span>{field.label}</span>
        {field.required && <span className="text-red-400">*</span>}
        {isSecret && (
          <span className="text-[9px] text-gray-600 uppercase tracking-wider">· encrypted</span>
        )}
      </label>
      {field.type === 'select' ? (
        <select
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        >
          <option value="">—</option>
          {(field.options || []).map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      ) : field.type === 'textarea' ? (
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={field.placeholder}
          rows={3}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        />
      ) : (
        <input
          type={field.type === 'password' ? 'password' : 'text'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={field.placeholder}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        />
      )}
      {field.description && <p className="text-[10px] text-gray-600 mt-0.5">{field.description}</p>}
    </div>
  )
}
