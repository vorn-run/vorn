import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import { useAppStore } from '../../stores'
import { SettingsPageHeader } from './SettingsPageHeader'
import { buildConnectorListings, type ConnectorListing } from '../../lib/connector-browse'
import { usePackInstall } from '../../lib/use-pack-install'
import { useConnectorCatalog, refreshConnectorCatalog } from '../../lib/use-connector-catalog'
import { refreshConnections } from '../../lib/use-connections'
import { SDK_FILTER_KEYS } from '../../../shared/types'
import { ConnectorDirectory } from './ConnectorDirectory'
import { ConnectorDetail } from './ConnectorDetail'
import { InstalledPlugins } from './InstalledPlugins'
import { ConnectionGroups, type ConnectorStatus } from './ConnectionGroups'
import { useRowAction } from '../../lib/use-row-action'
import { waitForSync } from '../../lib/connection-sync'
import type { InstalledConnectorPack, SourceConnection } from '../../../shared/types'
import { SdkConnectorForm } from './SdkConnectorForm'
import { PackInstallConfirm } from './PackInstallConfirm'
import { AddConnectionForm, MCP_CONNECTOR_ID, type ConnectorInfo } from './AddConnectionForm'
import { refreshExtensions } from '../../lib/use-extensions'
import { useClaimedTerminalIds } from '../../hooks/usePanelTerminals'

const TAB_LABEL = {
  installed: 'Installed',
  connections: 'Connections',
  browse: 'Browse'
} as const

export function ConnectorSettings() {
  const workflows = useAppStore((s) => s.config?.workflows ?? [])
  // Settings is a modal overlay, so it closes first or the editor renders behind it; null opens a new workflow.
  const openWorkflowEditor = (id: string | null) => {
    const store = useAppStore.getState()
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
  // One catalog for the whole app: the step library offers these same
  // connectors before they are installed, and two copies would disagree the
  // moment either was refreshed.
  const { items: catalog, mcpServers, fetchedAt: catalogFetchedAt } = useConnectorCatalog()
  // What the detail view is describing.
  const [selected, setSelected] = useState<ConnectorListing | null>(null)
  // What is installed leads: it is what a person came back for. With nothing on
  // disk there is nothing to lead with, so the catalog opens instead.
  const [view, setView] = useState<'installed' | 'connections' | 'browse'>('installed')
  const [packs, setPacks] = useState<InstalledConnectorPack[]>([])
  const activity = useRowAction()
  const [backfillResult, setBackfillResult] = useState<
    Record<string, { imported: number; updated: number; error?: string }>
  >({})

  // Decided once, from the first load: switching away from an empty view
  // mid-session because the last pack was removed would be the page moving
  // under someone's hands.
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
    if (!decidedView.current && installed.length === 0) setView('browse')
    decidedView.current = true
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: loads connectors from the main process on mount
    void load()
  }, [load])

  // A sign-in finishes in its own window, so the rows follow the server rather than the press.
  useEffect(
    () =>
      window.api.onConfigChanged?.(() => void window.api.listConnections().then(setConnections)),
    []
  )

  // Installing changes what is on disk, which the library reads too, so the shared cache is told as well.
  const install = usePackInstall(
    useCallback(async () => {
      await Promise.all([load(), refreshConnections()])
    }, [load])
  )
  const {
    progress: installProgress,
    pending: pendingPack,
    error: fileInstallError,
    installing: installingPending
  } = install
  // The row and the page both show the version, the sign-in and the receipt, so a pack that matches needs no sheet.
  const handleInstall = useCallback(
    (listing: ConnectorListing) => install.inspect(listing, { direct: true }),
    [install]
  )
  const handleInstallFile = install.inspectFile
  const handleConfirmPending = install.confirm
  // One sheet, shown wherever the install was asked for.
  const pendingSheet = pendingPack && (
    <PackInstallConfirm
      preview={pendingPack.preview}
      busy={installingPending}
      onConfirm={handleConfirmPending}
      onCancel={install.cancel}
    />
  )

  const handleRollback = useCallback(
    async (id: string) => {
      await activity.run('rollback', id, () => window.api.rollbackConnectorPack(id))
      await load()
    },
    [load, activity]
  )

  const handleRemovePack = useCallback(
    async (id: string) => {
      await activity.run('remove', id, async () => {
        const result = await window.api.removeConnectorPack(id)
        // What a card offers is read from this list, and one of them just left it.
        if (result.ok) void refreshExtensions()
        // Said after the fact rather than asked before it: the count is what the
        // server counted, and a connection left without files is worth naming.
        if (result.ok && (result.connections ?? 0) > 0) {
          install.report(
            `Removed the files. ${result.connections} connection${result.connections === 1 ? '' : 's'} will stop working until the connector is installed again.`
          )
        }
        return result
      })
      await load()
    },
    [load, install, activity]
  )

  const listings = useMemo(
    () => buildConnectorListings(connectors, catalog, connections, packs, mcpServers),
    [connectors, catalog, connections, packs, mcpServers]
  )
  // The map itself, which the store owns; counting inside the selector would
  // hand `useShallow` a fresh object every call and never settle.
  const activation = useAppStore((s) => s.extensionActivation)
  // A shell inside a card's panel is a session of its own and deliberately not a
  // card, so counting sessions would inflate both halves of the fraction.
  const claimed = useClaimedTerminalIds()
  // One pass for the whole panel, with the honest denominator beside it: a card
  // this window never opened cannot be counted, and a total would overstate it.
  const { activeCards, openCards } = useMemo(() => {
    const counts: Record<string, number> = {}
    let open = 0
    for (const [sessionId, states] of activation) {
      if (claimed.has(sessionId)) continue
      open += 1
      for (const state of states) {
        // Seeded either way, so an extension on no card says so rather than going quiet.
        counts[state.extensionId] ??= 0
        const draws = state.panes.length + state.footers.length + state.linkHandlers.length > 0
        if (state.active && draws) counts[state.extensionId] += 1
      }
    }
    return { activeCards: counts, openCards: open }
  }, [activation, claimed])
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

  const handleRun = async (workflowId: string, connectionId: string) => {
    const since = connections.find((connection) => connection.id === connectionId)?.lastSyncAt
    await activity.run('run', workflowId, async () => {
      await window.api.runWorkflowManual(workflowId)
      await waitForSync(connectionId, since)
    })
    load()
  }

  const handleReset = async (connectionId: string, event: string) => {
    await window.api.seedConnectorWorkflow(connectionId, event)
    load()
  }

  const handleBackfill = async (connectionId: string) => {
    setBackfillResult((prev) => {
      const { [connectionId]: _removed, ...rest } = prev
      return rest
    })
    await activity.run('backfill', connectionId, async () => {
      const result = await window.api.backfillConnection(connectionId)
      setBackfillResult((prev) => ({ ...prev, [connectionId]: result }))
    })
    load()
  }

  const handleDelete = async (connectionId: string) => {
    await activity.run('delete', connectionId, () => window.api.deleteConnection(connectionId))
    load()
  }

  return (
    <div>
      <SettingsPageHeader
        title="Plugins"
        description="Connectors watch a service and start workflows. Extensions add footers and panes to a card. Both install as packs from the same directory."
      />

      <div className="inline-flex bg-white/[0.04] rounded-sm p-0.5 mb-4">
        {(['installed', 'connections', 'browse'] as const).map((tab) => (
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
            {TAB_LABEL[tab]}
          </button>
        ))}
      </div>

      {view === 'installed' && !adding && !selectedListing && (
        <InstalledPlugins
          listings={listings}
          builtIns={connectors}
          progress={installProgress}
          activeCards={activeCards}
          openCards={openCards}
          activity={activity}
          pending={pendingPack ? { sheet: pendingSheet, rowKey: pendingPack.rowKey } : undefined}
          onSelect={setSelected}
          onInstall={handleInstall}
          onAdd={setAdding}
          onRemove={(listing) => handleRemovePack(listing.id)}
          onBrowse={() => setView('browse')}
        />
      )}

      {view === 'connections' && !adding && (
        <>
          <ConnectionGroups
            connections={connections}
            listings={listings}
            manifests={manifests}
            statuses={statuses}
            workflows={workflows}
            activity={activity}
            backfillResult={backfillResult}
            onAdd={setAdding}
            onRun={handleRun}
            onBackfill={handleBackfill}
            onDelete={handleDelete}
            onResetWorkflow={handleReset}
            onOpenWorkflow={openWorkflowEditor}
            onRefresh={load}
          />
          {connections.length === 0 && packs.length === 0 && (
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
          onRefresh={async () => {
            await refreshConnectorCatalog()
          }}
          activeCards={activeCards}
          onSelect={setSelected}
          onAdd={setAdding}
          onInstall={handleInstall}
          onInstallFile={handleInstallFile}
          onPickFile={() => window.api.openFileDialog()}
          installError={fileInstallError}
          pending={pendingPack ? { sheet: pendingSheet, rowKey: pendingPack.rowKey } : undefined}
        />
      )}

      {/* Opened from whichever tab was showing, and closing goes back to it. */}
      {view !== 'connections' && !adding && selectedListing && (
        <ConnectorDetail
          listing={selectedListing}
          backLabel={view === 'installed' ? 'All plugins' : 'All connectors'}
          builtIns={connectors}
          {...(installProgress[selectedListing.id] && {
            progress: installProgress[selectedListing.id]
          })}
          activity={activity.state(selectedListing.id, ['rollback', 'remove'])}
          {...(selectedListing.kind === 'extension' && {
            cards: { active: activeCards[selectedListing.id] ?? 0, open: openCards }
          })}
          pending={pendingPack?.rowKey === selectedListing.key ? pendingSheet : null}
          onAdd={() => setAdding(selectedListing)}
          onUse={() => openWorkflowEditor(null)}
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
