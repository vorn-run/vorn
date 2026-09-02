import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import { useAppStore } from '../../stores'
import { SettingsPageHeader } from './SettingsPageHeader'
import { buildConnectorListings, type ConnectorListing } from '../../lib/connector-browse'
import { usePackInstall } from '../../lib/use-pack-install'
import { SDK_FILTER_KEYS } from '../../../shared/types'
import { ConnectorDirectory } from './ConnectorDirectory'
import { ConnectorDetail } from './ConnectorDetail'
import { ConnectionGroups, type ConnectorStatus } from './ConnectionGroups'
import type {
  ConnectorCatalogItem,
  ConnectorCatalogSnapshot,
  InstalledConnectorPack,
  McpServerCatalogEntry,
  SourceConnection
} from '../../../shared/types'
import { SdkConnectorForm } from './SdkConnectorForm'
import { PackInstallConfirm } from './PackInstallConfirm'
import { AddConnectionForm, MCP_CONNECTOR_ID, type ConnectorInfo } from './AddConnectionForm'

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
  const [mcpServers, setMcpServers] = useState<McpServerCatalogEntry[]>([])
  const [catalogFetchedAt, setCatalogFetchedAt] = useState<number>()
  // What the detail view is describing. Separate from `adding` so opening a
  // connector to read about it is not the same as committing to install it.
  const [selected, setSelected] = useState<ConnectorListing | null>(null)
  // Connections lead once there are any; with none there is nothing to lead
  // with, so the catalog opens instead of a second empty-state layout.
  const [view, setView] = useState<'connections' | 'browse'>('connections')
  const [packs, setPacks] = useState<InstalledConnectorPack[]>([])
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
    setMcpServers(snapshot.mcpServers ?? [])
    setCatalogFetchedAt(snapshot.fetchedAt)
  }, [])

  useEffect(() => {
    void window.api.listConnectorCatalog().then(applyCatalog)
  }, [applyCatalog])

  // Inspect, ask, keep — the same three steps the editor's template rows use.
  const install = usePackInstall(load)
  const {
    progress: installProgress,
    pending: pendingPack,
    error: fileInstallError,
    installing: installingPending
  } = install
  const handleInstall = install.inspect
  const handleInstallFile = install.inspectFile
  const handleConfirmPending = install.confirm

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
        install.report(
          `Removed the files. ${result.connections} connection${result.connections === 1 ? '' : 's'} will stop working until the connector is installed again.`
        )
      }
      await load()
    },
    [load, install]
  )

  const listings = useMemo(
    () => buildConnectorListings(connectors, catalog, connections, packs, mcpServers),
    [connectors, catalog, connections, packs, mcpServers]
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
  // Every listed server is a connection to the built-in `mcp` connector.
  const mcpConnector = connectors.find((c) => c.id === MCP_CONNECTOR_ID)

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
                onCancel={install.cancel}
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

      {/* A generic server has no manifest to probe, so it goes to the manual
          form with its launch line already written. */}
      {adding?.mcpServer &&
        (mcpConnector ? (
          <AddConnectionForm
            connector={mcpConnector}
            startManual
            initialAuth={{
              command: adding.mcpServer.command,
              args: JSON.stringify(adding.mcpServer.args)
            }}
            // Names the server the connection belongs to, so its row counts it
            // rather than lumping it in with every other stdio connection.
            extraFilters={{ [SDK_FILTER_KEYS.connectorId]: adding.mcpServer.id }}
            onDone={() => {
              setAdding(null)
              setView('connections')
              load()
            }}
            onCancel={() => setAdding(null)}
          />
        ) : (
          <p className="p-4 text-[12px] text-danger border border-white/[0.08] rounded-sm">
            The MCP connector is not available in this build, so {adding.name} cannot be added here.
          </p>
        ))}
    </div>
  )
}
