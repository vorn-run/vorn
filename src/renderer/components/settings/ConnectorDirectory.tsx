import { useMemo, useState } from 'react'
import { Search, Plus, RefreshCw, ChevronRight } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import {
  describeCatalogAge,
  filterConnectorListings,
  filterByCategory,
  connectorCategories,
  listingDetails,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'

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
  fetchedAt,
  onRefresh
}: {
  listings: ConnectorListing[]
  builtIns: BuiltInConnector[]
  onSelect: (listing: ConnectorListing) => void
  onAdd: (listing: ConnectorListing) => void
  /** When the published list was last read. Absent until one has been. */
  fetchedAt?: number
  onRefresh?: () => Promise<void> | void
}) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('')
  const [refreshing, setRefreshing] = useState(false)

  const categories = useMemo(() => connectorCategories(listings), [listings])
  const visible = useMemo(
    () => filterByCategory(filterConnectorListings(listings, search), category || undefined),
    [listings, search, category]
  )

  return (
    <div>
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

      <div>
        {visible.map((listing) => (
          <ConnectorRow
            key={listing.key}
            listing={listing}
            builtIns={builtIns}
            onSelect={() => onSelect(listing)}
            onAdd={() => onAdd(listing)}
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

function ConnectorRow({
  listing,
  builtIns,
  onSelect,
  onAdd
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  onSelect: () => void
  onAdd: () => void
}) {
  const details = listingDetails(listing, builtIns)

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
          {/* Without this nothing says the row opens anything, and the Add
              button next to it reads as the only thing that does. */}
          <span className="inline-flex items-center gap-0.5 text-[11px] text-gray-500 group-hover:text-gray-300 transition-colors mt-1.5">
            Details <ChevronRight size={11} />
          </span>
        </span>
      </button>

      {/* An installed row has no manifest and no package spec, so there is
          nothing to open a form against. */}
      {listing.source !== 'installed' && (
        <button
          onClick={onAdd}
          className="shrink-0 self-center text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
        >
          <Plus size={11} /> {listing.connectedCount > 0 ? 'Add another' : 'Add'}
        </button>
      )}
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
