import { useMemo, useState } from 'react'
import { Search, Plus, RefreshCw, ChevronRight, Download, FolderOpen } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import type { ConnectorInstallProgress } from '../../../shared/types'
import {
  describeCatalogAge,
  filterConnectorListings,
  filterByCategory,
  connectorCategories,
  listingDetails,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'
import { canAddConnection, describePackStatus, packStateFor } from '../../lib/pack-status'
import { TONE_DOT, TONE_TEXT } from '../../lib/status-tone'

/**
 * The connectors Vorn can talk to, one per row.
 *
 * Rows rather than cards because this lives in a settings panel: two columns
 * halve the width available for a description and double the chrome around it.
 * Every catalog built to sit inside a panel is a single column with the facts
 * in one muted line and one action on the right.
 *
 * Those facts come from each connector's own manifest by way of the catalog, so
 * a row can answer "will this do what I need" before anything is downloaded.
 */
export function ConnectorDirectory({
  listings,
  builtIns,
  onSelect,
  onAdd,
  onInstall,
  onInstallFile,
  onPickFile,
  installError,
  progress,
  fetchedAt,
  onRefresh
}: {
  listings: ConnectorListing[]
  builtIns: BuiltInConnector[]
  onSelect: (listing: ConnectorListing) => void
  onAdd: (listing: ConnectorListing) => void
  onInstall?: (listing: ConnectorListing) => void
  /** A pack chosen from disk, by absolute path. */
  onInstallFile?: (filePath: string) => void
  /** Opens the file picker and answers with what was chosen. */
  onPickFile?: () => Promise<string | null>
  /** Why the last file install was refused, for the one that has no row yet. */
  installError?: string | null
  /** Installs running right now, by connector id. */
  progress?: Record<string, ConnectorInstallProgress>
  /** When the published list was last read. Absent until one has been. */
  fetchedAt?: number
  onRefresh?: () => Promise<void> | void
}) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('')
  const [refreshing, setRefreshing] = useState(false)
  const [dragOver, setDragOver] = useState(false)

  const categories = useMemo(() => connectorCategories(listings), [listings])
  const visible = useMemo(
    () => filterByCategory(filterConnectorListings(listings, search), category || undefined),
    [listings, search, category]
  )

  const handleDrop = (event: React.DragEvent): void => {
    event.preventDefault()
    event.stopPropagation()
    setDragOver(false)
    if (!onInstallFile) return
    for (const file of Array.from(event.dataTransfer?.files ?? [])) {
      const filePath = (file as File & { path?: string }).path
      if (filePath) onInstallFile(filePath)
    }
  }

  return (
    <div
      onDragOver={
        onInstallFile
          ? (event) => {
              event.preventDefault()
              setDragOver(true)
            }
          : undefined
      }
      onDragLeave={(event) => {
        event.preventDefault()
        setDragOver(false)
      }}
      onDrop={onInstallFile ? handleDrop : undefined}
      className={
        dragOver ? 'outline-dashed outline-1 outline-offset-4 outline-white/[0.25]' : undefined
      }
      data-drop-active={dragOver ? 'true' : undefined}
    >
      <div className="flex items-center gap-2 mb-1">
        <div className="relative flex-1">
          <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-600" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search connectors"
            className="w-full pl-7 pr-2 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-200 focus:border-white/[0.2] outline-none"
          />
        </div>
        {onInstallFile && onPickFile && (
          <button
            onClick={async () => {
              const filePath = await onPickFile()
              if (filePath) onInstallFile(filePath)
            }}
            title="Install a .vorn.tgz you already have"
            className="shrink-0 py-1.5 px-2 text-xs text-gray-300 hover:text-white border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
          >
            <FolderOpen size={11} /> Install from file
          </button>
        )}
        {/* A dropdown rather than a row of chips, which wrapped to a second
            line and pushed the list down. */}
        {categories.length > 1 && (
          <select
            value={category}
            onChange={(e) => setCategory(e.target.value)}
            aria-label="Filter by category"
            className="py-1.5 px-2 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-300 outline-none focus:border-white/[0.2]"
          >
            <option value="">All categories</option>
            {categories.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        )}
      </div>

      {installError && (
        <p className={`flex items-start gap-1.5 text-[11px] mt-2 ${TONE_TEXT.broken}`}>
          <span className={`w-1.5 h-1.5 rounded-full shrink-0 mt-1 ${TONE_DOT.broken}`} />
          {installError}
        </p>
      )}

      <div>
        {visible.map((listing) => (
          <ConnectorRow
            key={listing.key}
            listing={listing}
            builtIns={builtIns}
            {...(progress?.[listing.id] && { progress: progress[listing.id] })}
            onSelect={() => onSelect(listing)}
            onAdd={() => onAdd(listing)}
            {...(onInstall && { onInstall: () => onInstall(listing) })}
          />
        ))}
      </div>

      {visible.length === 0 && (
        <p className="text-sm text-gray-500 py-4">
          {search || category
            ? 'No connectors match that.'
            : 'No connectors available. Check your connection and try again.'}
        </p>
      )}

      {onRefresh && (
        <div className="flex items-center gap-1.5 pt-3 text-[11px] text-gray-600 border-t border-white/[0.06]">
          <span>{describeCatalogAge(fetchedAt)}</span>
          <span>·</span>
          <button
            onClick={async () => {
              setRefreshing(true)
              try {
                await onRefresh()
              } finally {
                setRefreshing(false)
              }
            }}
            disabled={refreshing}
            className="inline-flex items-center gap-1 text-gray-500 hover:text-gray-300 transition-colors disabled:opacity-50"
          >
            <RefreshCw size={9} className={refreshing ? 'animate-spin' : undefined} />
            {refreshing ? 'Checking' : 'Check now'}
          </button>
        </div>
      )}
    </div>
  )
}

export function ConnectorRow({
  listing,
  builtIns,
  progress,
  onSelect,
  onAdd,
  onInstall
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  /** The install running for this connector, when one is. */
  progress?: ConnectorInstallProgress
  onSelect: () => void
  onAdd: () => void
  onInstall?: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const state = packStateFor({
    installed: listing.pack,
    catalogVersion: listing.catalogItem?.version,
    progress
  })
  const status = describePackStatus(state)
  const installable = listing.source !== 'builtin' && onInstall !== undefined

  return (
    <div className="flex items-start gap-3 py-3 border-t border-white/[0.06]">
      <button
        onClick={onSelect}
        className="flex items-start gap-3 flex-1 min-w-0 text-left group"
        aria-label={`About ${listing.name}`}
      >
        <span className="w-8 h-8 shrink-0 flex items-center justify-center bg-white/[0.05] rounded-md mt-0.5">
          <ConnectorIcon
            connectorId={listing.id}
            icon={listing.catalogItem?.icon ?? listing.icon}
            size={17}
            className="text-gray-200"
          />
        </span>
        <span className="min-w-0">
          <span className="block text-[13.5px] text-gray-200 font-medium group-hover:underline underline-offset-2 decoration-white/25">
            {listing.name}
          </span>
          {listing.description && (
            <span className="block text-[12.5px] text-gray-500 leading-snug mt-0.5">
              {listing.description}
            </span>
          )}
          <span className="block text-[11px] text-gray-600 mt-1.5">{facts(listing, details)}</span>

          {status.percent !== null && (
            <span className="block h-px w-full max-w-[220px] bg-white/[0.08] mt-2 overflow-hidden">
              <span
                className="block h-px bg-ink transition-[width] duration-200"
                style={{ width: `${status.percent}%` }}
                role="progressbar"
                aria-valuenow={status.percent}
                aria-label={`Installing ${listing.name}`}
              />
            </span>
          )}

          {/* Colour is only spent here: a rejection, and the sage dot that says
              a connector is on disk. Everything else on this page stays muted. */}
          {state.kind !== 'absent' && (
            <span
              className={`flex items-center gap-1.5 text-[11px] mt-1.5 ${TONE_TEXT[status.tone]}`}
            >
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${TONE_DOT[status.tone]}`} />
              {status.detail ?? status.label}
            </span>
          )}

          {/* Without this nothing says the row opens anything, and the button
              next to it reads as the only thing that does. */}
          <span className="inline-flex items-center gap-0.5 text-[11px] text-gray-500 group-hover:text-gray-300 transition-colors mt-1.5">
            Details <ChevronRight size={11} />
          </span>
        </span>
      </button>

      <div className="shrink-0 self-center flex items-center gap-1.5">
        {installable && status.action && (
          <button
            onClick={onInstall}
            disabled={status.busy}
            className="text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
          >
            {status.action === 'install' ? <Download size={11} /> : <RefreshCw size={11} />}
            {status.label}
          </button>
        )}
        {/* A pack has to be on disk before there is anything to connect to; a
            built-in is already there. */}
        {canAddConnection(state, {
          source: listing.source,
          hasLegacyLaunch: Boolean(listing.catalogItem?.packageName)
        }) && (
          <button
            onClick={onAdd}
            className="text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
          >
            <Plus size={11} /> {listing.connectedCount > 0 ? 'Add another' : 'Add'}
          </button>
        )}
      </div>
    </div>
  )
}

/**
 * The one muted line under a connector's name.
 *
 * Written the way a good store writes them — "1 trigger, 1 action" — rather
 * than a coloured pill per fact. Colour stops meaning anything when every row
 * carries three of it, and the only thing on this page that earns colour is a
 * connection that is failing.
 */
function facts(listing: ConnectorListing, details: ReturnType<typeof listingDetails>): string {
  const parts = [listing.category]

  const offers = [
    details.triggers.length > 0 && count(details.triggers.length, 'trigger'),
    details.actions.length > 0 && count(details.actions.length, 'action')
  ].filter(Boolean) as string[]
  if (offers.length > 0) parts.push(offers.join(', '))

  const version = listing.catalogItem?.version
  if (version) parts.push(`v${version}`)
  if (listing.connectedCount > 0) parts.push('in use')

  return parts.join(' · ')
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
