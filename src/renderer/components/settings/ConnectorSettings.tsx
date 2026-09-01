import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import { useAppStore } from '../../stores'
import { SettingsPageHeader } from './SettingsPageHeader'
import { ConnectorIcon } from '../ConnectorIcon'
import { buildConnectorListings, type ConnectorListing } from '../../lib/connector-browse'
import { ConnectorDirectory } from './ConnectorDirectory'
import { ConnectorDetail } from './ConnectorDetail'
import { ConnectionGroups, type ConnectorStatus } from './ConnectionGroups'
import type {
  ConnectorCatalogItem,
  ConnectorCatalogSnapshot,
  ConnectorInstallProgress,
  ConnectorPackSource,
  ConnectorPackSummary,
  InstalledConnectorPack,
  SourceConnection,
  ConnectorManifest,
  TaskStatus
} from '../../../shared/types'
import { SdkConnectorForm } from './SdkConnectorForm'
import { PackInstallConfirm } from './PackInstallConfirm'
import { DynamicField } from './DynamicField'
import { Check, AlertCircle } from 'lucide-react'

/** Connector id the generic MCP stdio connection registers under. */
const MCP_CONNECTOR_ID = 'mcp'

interface ConnectorInfo {
  id: string
  name: string
  icon: string
  capabilities: string[]
  manifest: ConnectorManifest
}

/** Render text with `backtick` spans styled as inline code. */

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
  // One selection, so "both open at once" is not a representable state.
  const [adding, setAdding] = useState<ConnectorListing | null>(null)
  const [catalog, setCatalog] = useState<ConnectorCatalogItem[]>([])
  const [catalogFetchedAt, setCatalogFetchedAt] = useState<number>()
  // What the detail view is describing. Separate from `adding` so opening a
  // connector to read about it is not the same as committing to install it.
  const [selected, setSelected] = useState<ConnectorListing | null>(null)
  // Connections lead once there are any; with none there is nothing to lead
  // with, so the catalog opens instead of a second empty-state layout.
  const [view, setView] = useState<'connections' | 'browse'>('connections')
  const [packs, setPacks] = useState<InstalledConnectorPack[]>([])
  // Rejections live only here: nothing was written to disk, so they clear on reload.
  const [installProgress, setInstallProgress] = useState<Record<string, ConnectorInstallProgress>>(
    {}
  )
  // A file install has no row to fail on until its manifest is read.
  const [fileInstallError, setFileInstallError] = useState<string | null>(null)
  const [pendingPack, setPendingPack] = useState<{
    source: ConnectorPackSource
    preview: ConnectorPackSummary
  } | null>(null)
  const [installingPending, setInstallingPending] = useState(false)
  const [runningId, setRunningId] = useState<string | null>(null)
  const [backfillingId, setBackfillingId] = useState<string | null>(null)
  const [backfillResult, setBackfillResult] = useState<
    Record<string, { imported: number; updated: number; error?: string }>
  >({})

  // Decided once, from the first load: switching away from an empty
  // connections view mid-session because the last one was deleted would be the
  // page moving under someone's hands.
  const decidedView = useRef(false)

  const load = useCallback(async () => {
    const [c, conns, st, installed] = await Promise.all([
      window.api.listConnectors(),
      window.api.listConnections(),
      window.api.getConnectorStatus(),
      window.api.listConnectorPacks()
    ])
    setConnectors(c)
    setConnections(conns)
    setStatuses(st)
    setPacks(installed)
    if (!decidedView.current && conns.length === 0) setView('browse')
    decidedView.current = true
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: loads connectors from the main process on mount
    void load()
  }, [load])

  // Fetched once rather than with every refresh: the catalog is fixed for the
  // life of the process, so re-reading it after each run, delete or install
  // would be a round trip that always returns the same answer.
  const applyCatalog = useCallback((snapshot: ConnectorCatalogSnapshot) => {
    setCatalog(snapshot.items)
    setCatalogFetchedAt(snapshot.fetchedAt)
  }, [])

  useEffect(() => {
    void window.api.listConnectorCatalog().then(applyCatalog)
  }, [applyCatalog])

  // The unsubscribe is what keeps a reopened panel from stacking a second listener.
  useEffect(() => {
    return window.api.onConnectorInstallProgress((progress) => {
      setInstallProgress((current) => ({ ...current, [progress.id]: progress }))
    })
  }, [])

  // Every install is checked and shown first, so a catalog row and a dropped
  // file ask the same question before any of it is kept.
  const handleInstall = useCallback(
    async (listing: ConnectorListing, source?: ConnectorPackSource) => {
      const resolved =
        source ??
        (listing.catalogItem?.packUrl
          ? ({
              kind: 'url',
              url: listing.catalogItem.packUrl,
              ...(listing.catalogItem.sha256 && { sha256: listing.catalogItem.sha256 })
            } as ConnectorPackSource)
          : ({ kind: 'npm', packageName: listing.catalogItem?.packageName ?? listing.id } as const))

      setInstallProgress((current) => {
        const next = { ...current }
        delete next[listing.id]
        return next
      })
      const result = await window.api.inspectConnectorPack(resolved)
      if (!result.ok) {
        // Keyed by the row that asked, which is the row that shows the refusal.
        setInstallProgress((current) => ({
          ...current,
          [listing.id]: { id: listing.id, phase: 'failed', error: result.error }
        }))
        return
      }
      setPendingPack({ source: { kind: 'staged', token: result.preview.token }, preview: result.preview })
    },
    []
  )

  // Verified first and installed only on confirm, so a drop is a question.
  const handleInstallFile = useCallback(async (filePath: string) => {
    setFileInstallError(null)
    setPendingPack(null)
    const result = await window.api.inspectConnectorPack({ kind: 'file', path: filePath })
    if (!result.ok) {
      setFileInstallError(result.error)
      return
    }
    setPendingPack({
      source: { kind: 'staged', token: result.preview.token },
      preview: result.preview
    })
  }, [])

  // Installs the files the sheet described, not the source they came from.
  const handleConfirmPending = useCallback(async () => {
    if (!pendingPack) return
    const id = pendingPack.preview.id
    setInstallingPending(true)
    const result = await window.api.installConnectorPack(pendingPack.source)
    setInstallingPending(false)
    setPendingPack(null)
    if (result.ok) {
      setInstallProgress((current) => {
        const next = { ...current }
        delete next[id]
        return next
      })
    } else {
      setInstallProgress((current) => ({
        ...current,
        [id]: { id, phase: 'failed', error: result.error }
      }))
      setFileInstallError(result.error)
    }
    await load()
  }, [pendingPack, load])

  const handleRollback = useCallback(
    async (id: string) => {
      await window.api.rollbackConnectorPack(id)
      await load()
    },
    [load]
  )

  const handleRemovePack = useCallback(
    async (id: string) => {
      const result = await window.api.removeConnectorPack(id)
      // Said after the fact rather than asked before it: the count is what the
      // server counted, and a connection left without files is worth naming.
      if (result.ok && (result.connections ?? 0) > 0) {
        setFileInstallError(
          `Removed the files. ${result.connections} connection${result.connections === 1 ? '' : 's'} will stop working until the connector is installed again.`
        )
      }
      await load()
    },
    [load]
  )

  const listings = useMemo(
    () => buildConnectorListings(connectors, catalog, connections, packs),
    [connectors, catalog, connections, packs]
  )
  // Re-read from the current listings so a connection made while the panel is
  // open updates its "connected" count rather than showing the stale copy.
  const selectedListing = selected
    ? (listings.find((listing) => listing.key === selected.key) ?? selected)
    : null
  // Keyed by connector id so a group can reach the manifest for its filters
  // and its default workflows without searching the array per row.
  const manifests = useMemo(
    () => Object.fromEntries(connectors.map((c) => [c.id, c.manifest])),
    [connectors]
  )
  // Resolved up front so the built-in form is only rendered once there is a
  // connector to hand it.
  const addingBuiltIn =
    adding?.source === 'builtin' ? connectors.find((c) => c.id === adding.id) : undefined

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
        title="Connections"
        description="Vorn watches these and starts a workflow when something happens. Each connection seeds a visible, editable workflow that polls on cron."
      />

      <div className="inline-flex bg-white/[0.04] rounded-sm p-0.5 mb-4">
        {(['connections', 'browse'] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => {
              setView(tab)
              setSelected(null)
              setAdding(null)
            }}
            aria-pressed={view === tab}
            className={`text-[12px] px-3 py-1 rounded-sm transition-colors ${
              view === tab ? 'bg-white/[0.08] text-gray-200' : 'text-gray-500 hover:text-gray-300'
            }`}
          >
            {tab === 'connections' ? 'Your connections' : 'Browse'}
          </button>
        ))}
      </div>

      {view === 'connections' && !adding && (
        <>
          <ConnectionGroups
            connections={connections}
            listings={listings}
            manifests={manifests}
            statuses={statuses}
            workflows={workflows}
            runningId={runningId}
            backfillingId={backfillingId}
            backfillResult={backfillResult}
            onAdd={setAdding}
            onRun={handleRun}
            onBackfill={handleBackfill}
            onDelete={handleDelete}
            onResetWorkflow={handleReset}
            onOpenWorkflow={openWorkflowEditor}
            onRefresh={load}
          />
          {connections.length === 0 && (
            <p className="text-sm text-gray-500">
              No connections yet. Browse the connectors Vorn can talk to.
            </p>
          )}
        </>
      )}

      {view === 'browse' && !adding && !selectedListing && (
        <ConnectorDirectory
          listings={listings}
          builtIns={connectors}
          progress={installProgress}
          fetchedAt={catalogFetchedAt}
          onRefresh={async () => applyCatalog(await window.api.refreshConnectorCatalog())}
          onSelect={setSelected}
          onAdd={setAdding}
          onInstall={handleInstall}
          onInstallFile={handleInstallFile}
          onPickFile={() => window.api.openFileDialog()}
          installError={fileInstallError}
          {...(pendingPack && {
            pending: (
              <PackInstallConfirm
                preview={pendingPack.preview}
                busy={installingPending}
                onConfirm={handleConfirmPending}
                onCancel={() => setPendingPack(null)}
              />
            )
          })}
        />
      )}

      {view === 'browse' && !adding && selectedListing && (
        <ConnectorDetail
          listing={selectedListing}
          builtIns={connectors}
          {...(installProgress[selectedListing.id] && {
            progress: installProgress[selectedListing.id]
          })}
          onAdd={() => setAdding(selectedListing)}
          onInstall={() => handleInstall(selectedListing)}
          onRollback={() => handleRollback(selectedListing.id)}
          onRemove={() => handleRemovePack(selectedListing.id)}
          onClose={() => setSelected(null)}
        />
      )}

      {/* A side-loaded pack has no catalog entry but does have files to probe. */}
      {(adding?.catalogItem || adding?.pack) && (
        <div className="p-4 bg-white/[0.03] border border-white/[0.08] rounded-sm">
          <h4 className="text-sm text-gray-200 font-medium mb-3">Add {adding.name} connection</h4>
          <SdkConnectorForm
            {...(adding.catalogItem && { catalogEntry: adding.catalogItem })}
            {...(adding.pack && { pack: adding.pack })}
            onDone={() => {
              setAdding(null)
              setView('connections')
              load()
            }}
            onCancel={() => setAdding(null)}
          />
        </div>
      )}

      {adding && addingBuiltIn && (
        <AddConnectionForm
          connector={addingBuiltIn}
          onDone={() => {
            setAdding(null)
            setView('connections')
            load()
          }}
          onCancel={() => setAdding(null)}
        />
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
          auth.profileName?.trim() ||
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
