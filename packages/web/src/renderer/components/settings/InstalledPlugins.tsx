import { useMemo } from 'react'
import { ChevronRight, Download, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import type { ConnectorInstallProgress, ConnectorKind } from '../../../shared/types'
import {
  listingDetails,
  CONNECTOR_KIND,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'
import { count, describeContributions } from '../../lib/extension-copy'
import { describePackStatus, packStateFor } from '../../lib/pack-status'
import type { RowActivity } from '../../lib/use-row-action'
import { TONE_DOT, TONE_TEXT } from '../../lib/status-tone'
import { ActivityLine } from './ActivityLine'
import { BusyIcon } from './BusyIcon'

/**
 * What is on this machine, before what could be.
 *
 * The catalog answers "what is there"; this answers "what did I install, and is
 * it doing anything". They were one tab, and an extension fell through the gap:
 * it makes no connection, so the connections list never showed it and the only
 * way to find it was to browse the whole catalog for something already owned.
 *
 * Grouped by kind rather than category, because the question that brings a
 * person here is which sort of thing they are looking for, and the two answer
 * different questions — one watches a service, one draws on a card.
 */
export function InstalledPlugins({
  listings,
  builtIns,
  progress,
  activeCards,
  openCards,
  activity,
  pending,
  onSelect,
  onInstall,
  onAdd,
  onRemove,
  onBrowse
}: {
  listings: ConnectorListing[]
  builtIns: BuiltInConnector[]
  /** Installs running right now, by connector id. */
  progress?: Record<string, ConnectorInstallProgress>
  /** How many open cards each extension is active on, by extension id. */
  activeCards?: Record<string, number>
  /** Cards this window has open at all, the honest denominator. */
  openCards?: number
  activity: RowActivity
  /** The confirm sheet for a pack being updated, and the row it opens under. */
  pending?: { sheet: React.ReactNode; rowKey?: string }
  onSelect: (listing: ConnectorListing) => void
  onInstall?: (listing: ConnectorListing) => void
  onAdd: (listing: ConnectorListing) => void
  onRemove: (listing: ConnectorListing) => void
  onBrowse: () => void
}) {
  // Only what has files: a catalog row is something to install, not something installed.
  const installed = useMemo(() => listings.filter((listing) => listing.pack), [listings])
  const groups = useMemo(() => {
    const order: ConnectorKind[] = ['extension', 'connector']
    return order
      .map((kind) => ({ kind, listings: installed.filter((listing) => listing.kind === kind) }))
      .filter((group) => group.listings.length > 0)
  }, [installed])

  if (installed.length === 0) {
    return (
      <p className="text-sm text-gray-500 py-4">
        Nothing is installed yet.{' '}
        <button onClick={onBrowse} className="text-gray-300 hover:text-white underline">
          Browse
        </button>{' '}
        for a connector or an extension.
      </p>
    )
  }

  return (
    <div>
      {groups.map((group) => (
        <div key={group.kind}>
          {/* Headings earn their keep once both kinds are here; one alone says nothing. */}
          {groups.length > 1 && (
            <h3 className="text-[10px] uppercase tracking-[0.08em] text-gray-600 pt-4 pb-1">
              {CONNECTOR_KIND[group.kind].label}
            </h3>
          )}
          {group.listings.map((listing) => (
            <div key={listing.key}>
              <InstalledRow
                listing={listing}
                builtIns={builtIns}
                {...(progress?.[listing.id] && { progress: progress[listing.id] })}
                {...(activeCards?.[listing.id] !== undefined && {
                  activeCards: activeCards[listing.id]
                })}
                {...(openCards !== undefined && { openCards })}
                activity={activity}
                onSelect={() => onSelect(listing)}
                {...(onInstall && { onInstall: () => onInstall(listing) })}
                onAdd={() => onAdd(listing)}
                onRemove={() => onRemove(listing)}
              />
              {listing.key === pending?.rowKey && <div className="mt-2 mb-3">{pending.sheet}</div>}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}

export function InstalledRow({
  listing,
  builtIns,
  progress,
  activeCards,
  openCards,
  activity,
  onSelect,
  onInstall,
  onAdd,
  onRemove
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  progress?: ConnectorInstallProgress
  activeCards?: number
  openCards?: number
  activity: RowActivity
  onSelect: () => void
  onInstall?: () => void
  onAdd: () => void
  onRemove: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const state = packStateFor({
    installed: listing.pack,
    catalogItem: listing.catalogItem,
    progress
  })
  const status = describePackStatus(state)
  const rowActivity = activity.state(listing.id, ['remove'])
  const busy = Boolean(rowActivity.phrase)
  const extension = listing.kind === 'extension'

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
            {extension && (
              <span className="shrink-0 text-[10px] text-gray-500 border border-white/[0.1] rounded-sm px-1.5 py-px">
                {CONNECTOR_KIND.extension.badge}
              </span>
            )}
          </span>
          <span className="block text-[12.5px] text-gray-500 leading-snug mt-0.5">
            {offers(listing, details)}
          </span>
          <span className="block text-[11px] text-gray-600 mt-1.5">
            {facts(listing, state, activeCards, openCards)}
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

          {/* The one coloured line: an update waiting, or an install that was refused. */}
          {status.tone !== 'settled' && status.detail && (
            <span
              className={`flex items-center gap-1.5 text-[11px] mt-1.5 ${TONE_TEXT[status.tone]}`}
            >
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${TONE_DOT[status.tone]}`} />
              {status.detail}
            </span>
          )}

          <ActivityLine {...rowActivity} className="text-[11px] mt-1.5" />

          <span className="inline-flex items-center gap-0.5 text-[11px] text-gray-500 group-hover:text-gray-300 transition-colors mt-1.5">
            Details <ChevronRight size={11} />
          </span>
        </span>
      </button>

      <div className="shrink-0 self-center flex items-center gap-1.5">
        {onInstall && status.action && (
          <button
            onClick={onInstall}
            disabled={status.busy}
            title={
              status.action === 'update' ? 'Install the newer version' : 'Try installing again'
            }
            className="text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
          >
            {status.action === 'install' ? <Download size={11} /> : <RefreshCw size={11} />}
            {status.label}
          </button>
        )}
        {/* An extension shows where its rule names; only a connector waits on a connection. */}
        {!extension && !listing.implicitlyConnected && listing.connectedCount === 0 && (
          <button
            onClick={onAdd}
            title="Connect this connector to an account"
            className="text-[11.5px] text-gray-300 hover:text-white px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1"
          >
            <Plus size={11} /> Add connection
          </button>
        )}
        <button
          onClick={onRemove}
          disabled={busy}
          title="Delete the installed files"
          className="text-[11.5px] text-gray-400 hover:text-gray-200 px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
        >
          <BusyIcon busy={busy} icon={Trash2} size={11} /> Uninstall
        </button>
      </div>
    </div>
  )
}

/** What the thing offers, in its own terms: surfaces for one, triggers and actions for the other. */
function offers(listing: ConnectorListing, details: ReturnType<typeof listingDetails>): string {
  if (listing.kind === 'extension') {
    return describeContributions(listing.contributes) || 'Adds no surface yet'
  }
  const parts = [
    details.triggers.length > 0 && count(details.triggers.length, 'trigger'),
    details.actions.length > 0 && count(details.actions.length, 'action')
  ].filter(Boolean) as string[]
  return parts.length > 0 ? parts.join(', ') : 'Nothing declared yet'
}

/**
 * The muted line: what is on disk, then what it is doing.
 *
 * An installed row leads with its own version rather than the catalog's, which
 * is the number a person came to check. What it is doing differs by kind, and
 * saying "no connection yet" or "on no open card" beats saying nothing — a row
 * that goes quiet reads as broken.
 */
function facts(
  listing: ConnectorListing,
  state: ReturnType<typeof packStateFor>,
  activeCards?: number,
  openCards?: number
): string {
  const parts: string[] = []
  if (listing.pack) parts.push(`v${listing.pack.version}`)
  if (state.kind === 'installed' && state.availableVersion) {
    parts.push(`v${state.availableVersion} available`)
  }
  if (listing.kind === 'extension') {
    if (activeCards !== undefined) {
      parts.push(
        activeCards > 0
          ? `on ${activeCards} of ${count(openCards ?? activeCards, 'open card')}`
          : 'on no open card'
      )
    }
  } else if (listing.connectedCount > 0) {
    parts.push(count(listing.connectedCount, 'connection'))
  } else if (listing.implicitlyConnected) parts.push('ready')
  else parts.push('no connection yet')
  return parts.join(' · ')
}
