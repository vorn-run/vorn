import { useMemo, useState } from 'react'
import { Search, Check, Zap, Play, RefreshCw } from 'lucide-react'
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
 * The connector directory: search, facets, and a card per connector.
 *
 * A row per connector was proportionate when there was one packaged connector.
 * Connectors now arrive from their own repository on their own schedule, so the
 * question stops being "which of these two" and becomes "will this one do what
 * I need" — which a name and a blurb cannot answer. Each card carries what the
 * connector says about itself, and selecting one opens the rest.
 */
export function ConnectorDirectory({
  listings,
  builtIns,
  selectedKey,
  onSelect,
  fetchedAt,
  onRefresh
}: {
  listings: ConnectorListing[]
  builtIns: BuiltInConnector[]
  selectedKey?: string
  onSelect: (listing: ConnectorListing) => void
  /** When the published list was last read. Absent until one has been. */
  fetchedAt?: number
  onRefresh?: () => Promise<void> | void
}) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState<string>()
  const [refreshing, setRefreshing] = useState(false)

  const categories = useMemo(() => connectorCategories(listings), [listings])
  const visible = useMemo(
    () => filterByCategory(filterConnectorListings(listings, search), category),
    [listings, search, category]
  )

  return (
    <div>
      <div className="flex items-center justify-between gap-3 mb-3">
        <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider">Connectors</h3>
        <div className="relative w-56">
          <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-600" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search connectors"
            className="w-full pl-7 pr-2 py-1 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-200 focus:border-white/[0.2] outline-none"
          />
        </div>
      </div>

      {onRefresh && (
        <p className="flex items-center gap-1.5 text-[10.5px] text-gray-600 mb-3">
          <span>{describeCatalogAge(fetchedAt)}</span>
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
        </p>
      )}

      {/* Chips only earn their place once there is more than one thing to pick. */}
      {categories.length > 1 && (
        <div className="flex flex-wrap gap-1.5 mb-3">
          <Chip label="All" active={!category} onClick={() => setCategory(undefined)} />
          {categories.map((name) => (
            <Chip
              key={name}
              label={name}
              active={category === name}
              onClick={() => setCategory(category === name ? undefined : name)}
            />
          ))}
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
        {visible.map((listing) => (
          <ConnectorCard
            key={listing.key}
            listing={listing}
            builtIns={builtIns}
            selected={listing.key === selectedKey}
            onSelect={() => onSelect(listing)}
          />
        ))}
      </div>

      {visible.length === 0 && (
        <p className="text-sm text-gray-500">
          {search || category
            ? 'No connectors match that.'
            : 'No connectors available. Check your connection and try again.'}
        </p>
      )}
    </div>
  )
}

function Chip({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-pressed={active}
      className={`text-[11px] px-2.5 py-1 rounded-full border transition-colors ${
        active
          ? 'bg-white/[0.1] text-gray-200 border-transparent'
          : 'text-gray-500 border-white/[0.1] hover:text-gray-300'
      }`}
    >
      {label}
    </button>
  )
}

function ConnectorCard({
  listing,
  builtIns,
  selected,
  onSelect
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  selected: boolean
  onSelect: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const version = listing.catalogItem?.version

  return (
    <button
      onClick={onSelect}
      aria-current={selected}
      className={`text-left p-3 bg-white/[0.03] border rounded-sm transition-colors ${
        selected ? 'border-cyan-400/50' : 'border-white/[0.06] hover:border-white/[0.14]'
      }`}
    >
      <div className="flex items-start gap-2.5">
        <span className="w-7 h-7 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
          <ConnectorIcon
            connectorId={listing.id}
            icon={listing.catalogItem?.icon ?? listing.icon}
            size={16}
            className="text-gray-200"
          />
        </span>
        <span className="min-w-0">
          <span className="block text-[13px] text-gray-200 font-medium truncate">
            {listing.name}
          </span>
          <span className="block text-[10px] text-gray-600">
            {listing.category}
            {version && ` · v${version}`}
          </span>
        </span>
      </div>

      {listing.description && (
        <p className="text-[11.5px] text-gray-500 leading-snug mt-2.5">{listing.description}</p>
      )}

      <div className="flex items-center flex-wrap gap-1.5 mt-2.5">
        {/* Counts, not names: the card says whether it is worth opening, the
            panel says what it actually does. */}
        {details.triggers.length > 0 && (
          <Tag className="text-cyan-300/90 bg-cyan-400/10">
            <Zap size={9} /> {count(details.triggers.length, 'trigger')}
          </Tag>
        )}
        {details.actions.length > 0 && (
          <Tag className="text-amber-300/90 bg-amber-400/10">
            <Play size={9} /> {count(details.actions.length, 'action')}
          </Tag>
        )}
        {listing.connectedCount > 0 && (
          <Tag className="text-green-400 bg-green-400/10">
            <Check size={9} /> {listing.connectedCount} connected
          </Tag>
        )}
      </div>
    </button>
  )
}

function Tag({ className, children }: { className: string; children: React.ReactNode }) {
  return (
    <span
      className={`inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-sm ${className}`}
    >
      {children}
    </span>
  )
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
