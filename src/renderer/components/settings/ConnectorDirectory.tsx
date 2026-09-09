import { Fragment, useMemo, useState } from 'react'
import { Search, Plus, RefreshCw, ChevronRight, Check, Download, FolderOpen } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import type {
  ConnectorAuthRung,
  ConnectorInstallProgress,
  ConnectorKind
} from '../../../shared/types'
import {
  describeCatalogAge,
  filterConnectorListings,
  filterByAuthRung,
  filterByCategory,
  filterByKind,
  connectorAuthRungs,
  connectorCategories,
  connectorKinds,
  groupListingsByCategory,
  listingDetails,
  AUTH_RUNG,
  CONNECTOR_KIND,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'
import { describeContributions } from '../../lib/extension-copy'
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
  pending,
  progress,
  activeCards,
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
  /** The confirm sheet for a verified pack, and the listing it opens under when one was pressed. */
  pending?: { sheet: React.ReactNode; rowKey?: string }
  /** Installs running right now, by connector id. */
  progress?: Record<string, ConnectorInstallProgress>
  /** How many open cards each installed extension is active on, by extension id. */
  activeCards?: Record<string, number>
  /** When the published list was last read. Absent until one has been. */
  fetchedAt?: number
  onRefresh?: () => Promise<void> | void
}) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('')
  const [rung, setRung] = useState<'' | ConnectorAuthRung>('')
  const [kind, setKind] = useState<'' | ConnectorKind>('')
  const [refreshing, setRefreshing] = useState(false)
  const [dragOver, setDragOver] = useState(false)

  const categories = useMemo(() => connectorCategories(listings), [listings])
  const rungs = useMemo(() => connectorAuthRungs(listings), [listings])
  const kinds = useMemo(() => connectorKinds(listings), [listings])
  const visible = useMemo(
    () =>
      filterByKind(
        filterByAuthRung(
          filterByCategory(filterConnectorListings(listings, search), category || undefined),
          rung || undefined
        ),
        kind || undefined
      ),
    [listings, search, category, rung, kind]
  )
  // Sections earn their keep once there is more than one; below that they are
  // a heading over the whole list, which says nothing.
  const sections = useMemo(() => groupListingsByCategory(visible), [visible])
  const pendingKey =
    pending?.rowKey && visible.some((listing) => listing.key === pending.rowKey)
      ? pending.rowKey
      : undefined

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
        {/* A connector and an extension answer different questions, so which
            one you came for narrows the list before anything else does. */}
        {kinds.length > 1 && (
          <select
            value={kind}
            onChange={(e) => setKind(e.target.value as '' | ConnectorKind)}
            aria-label="Filter by kind"
            className="py-1.5 px-2 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-300 outline-none focus:border-white/[0.2]"
          >
            <option value="">Everything</option>
            {kinds.map((name) => (
              <option key={name} value={name}>
                {CONNECTOR_KIND[name].label}
              </option>
            ))}
          </select>
        )}
        {/* What setting one up will ask of you is the question people bring
            here second, right after what it talks to. */}
        {rungs.length > 1 && (
          <select
            value={rung}
            onChange={(e) => setRung(e.target.value as '' | ConnectorAuthRung)}
            aria-label="Filter by sign-in"
            className="py-1.5 px-2 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-300 outline-none focus:border-white/[0.2]"
          >
            <option value="">Any sign-in</option>
            {rungs.map((name) => (
              <option key={name} value={name}>
                {AUTH_RUNG[name].label}
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

      {/* A verified pack with no row on screen, such as one dropped as a file. */}
      {pending && !pendingKey && <div className="mt-2">{pending.sheet}</div>}

      {sections.map((section) => (
        <div key={section.category}>
          {sections.length > 1 && (
            <h3 className="text-[10px] uppercase tracking-[0.08em] text-gray-600 pt-4 pb-1">
              {section.category}
            </h3>
          )}
          {section.listings.map((listing) => (
            <Fragment key={listing.key}>
              <ConnectorRow
                listing={listing}
                builtIns={builtIns}
                {...(progress?.[listing.id] && { progress: progress[listing.id] })}
                {...(activeCards?.[listing.id] !== undefined && {
                  activeCards: activeCards[listing.id]
                })}
                onSelect={() => onSelect(listing)}
                onAdd={() => onAdd(listing)}
                {...(onInstall && { onInstall: () => onInstall(listing) })}
              />
              {listing.key === pendingKey && <div className="mt-2 mb-3">{pending?.sheet}</div>}
            </Fragment>
          ))}
        </div>
      ))}

      {visible.length === 0 && (
        <p className="text-sm text-gray-500 py-4">
          {search || category || rung || kind
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
  activeCards,
  onSelect,
  onAdd,
  onInstall
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  /** The install running for this connector, when one is. */
  progress?: ConnectorInstallProgress
  /** Open cards this extension is active on, when it is installed. */
  activeCards?: number
  onSelect: () => void
  onAdd: () => void
  onInstall?: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const state = packStateFor({
    installed: listing.pack,
    catalogItem: listing.catalogItem,
    progress
  })
  const status = describePackStatus(state)
  // An MCP server is a command this machine runs, so there is no pack to install.
  const installable =
    listing.source !== 'builtin' && listing.source !== 'mcp' && onInstall !== undefined

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
          <span className="flex items-center gap-1.5 min-w-0">
            <span className="text-[13.5px] text-gray-200 font-medium group-hover:underline underline-offset-2 decoration-white/25 truncate">
              {listing.name}
            </span>
            {/* Both are facts rather than states, so they are lettered, not coloured. */}
            {listing.kind === 'extension' && (
              <span className="shrink-0 text-[10px] text-gray-500 border border-white/[0.1] rounded-sm px-1.5 py-px">
                {CONNECTOR_KIND.extension.badge}
              </span>
            )}
            {listing.authRung && (
              <span className="shrink-0 text-[10px] text-gray-500 border border-white/[0.1] rounded-sm px-1.5 py-px">
                {AUTH_RUNG[listing.authRung].badge}
              </span>
            )}
            {listing.verified && (
              <span
                title={`Checked ${listing.verified.checks.join(', ')} against v${listing.verified.version}`}
                className="shrink-0 inline-flex items-center gap-0.5 text-[10px] text-gray-500 border border-white/[0.1] rounded-sm px-1.5 py-px"
              >
                <Check size={9} /> verified
              </span>
            )}
          </span>
          {listing.description && (
            <span className="block text-[12.5px] text-gray-500 leading-snug mt-0.5">
              {listing.description}
            </span>
          )}
          <span className="block text-[11px] text-gray-600 mt-1.5">
            {facts(listing, details, activeCards)}
          </span>

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

          {/* The only colour on the row: a rejection, an unreleased entry, or the dot saying it is on disk. */}
          {state.kind !== 'absent' && (
            <span
              className={`flex items-center gap-1.5 text-[11px] mt-1.5 ${TONE_TEXT[status.tone]}`}
            >
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${TONE_DOT[status.tone]}`} />
              {/* A row has room for the label; the whole sentence belongs on the page. */}
              {state.kind === 'not-released' ? status.label : (status.detail ?? status.label)}
            </span>
          )}

          {/* Without this nothing says the row opens anything. */}
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
        {/* A pack must be on disk before there is anything to connect to, and a
            connector that signs in with nothing was connected by installing it. */}
        {!listing.implicitlyConnected &&
          canAddConnection(state, {
            source: listing.source,
            kind: listing.kind,
            hasLegacyLaunch: Boolean(listing.catalogItem?.packageName)
          }) && (
            <button
              onClick={onAdd}
              className="text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
            >
              <Plus size={11} />{' '}
              {listing.connectedCount > 0
                ? 'Add another'
                : listing.source === 'mcp'
                  ? 'Add server'
                  : 'Add'}
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
function facts(
  listing: ConnectorListing,
  details: ReturnType<typeof listingDetails>,
  activeCards?: number
): string {
  const parts = [listing.category]

  // An extension adds to a card rather than firing a workflow, so it counts what it adds.
  const offers =
    listing.kind === 'extension'
      ? [describeContributions(listing.contributes)].filter(Boolean)
      : ([
          details.triggers.length > 0 && count(details.triggers.length, 'trigger'),
          details.actions.length > 0 && count(details.actions.length, 'action')
        ].filter(Boolean) as string[])
  if (offers.length > 0) parts.push(offers.join(', '))

  const version = listing.catalogItem?.version ?? listing.pack?.version
  if (version) parts.push(`v${version}`)
  if (listing.kind === 'extension') {
    // Only worth saying once it is installed; before that there are no cards to be on.
    if (listing.pack && activeCards !== undefined) {
      parts.push(activeCards > 0 ? `on ${count(activeCards, 'card')}` : 'on no open card')
    }
  } else if (listing.connectedCount > 0) parts.push('in use')
  // Nothing was connected, and nothing needs to be: it is usable as it stands.
  else if (listing.implicitlyConnected) parts.push('ready')

  return parts.join(' · ')
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
