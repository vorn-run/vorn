import { Plus, X, ExternalLink, KeyRound } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import {
  listingDetails,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'

/**
 * What a connector does, before anything is downloaded.
 *
 * Every string here comes from the connector's own manifest — through the
 * catalog for a packaged one, directly for a built-in — so it cannot advertise
 * a trigger that has since been renamed. None of it is trusted once Add is
 * pressed: the connection form probes the package actually installed, so what
 * gets configured always matches what arrived.
 */
export function ConnectorDetail({
  listing,
  builtIns,
  onAdd,
  onClose
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  onAdd: () => void
  onClose: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const entry = listing.catalogItem
  const required = details.settings.filter((setting) => setting.required)
  const optional = details.settings.filter((setting) => !setting.required)

  return (
    <div className="mt-2 p-4 bg-white/[0.02] border border-white/[0.08] rounded-sm">
      <div className="flex items-start gap-3">
        <span className="w-8 h-8 shrink-0 flex items-center justify-center bg-white/[0.04] rounded-sm">
          <ConnectorIcon
            connectorId={listing.id}
            icon={entry?.icon ?? listing.icon}
            size={18}
            className="text-gray-200"
          />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-sm text-gray-200 font-medium">{listing.name}</div>
          <div className="text-[11px] text-gray-600 truncate">
            {entry?.packageName ?? (listing.source === 'builtin' ? 'Built in' : listing.id)}
            {entry?.version && ` · v${entry.version}`}
            {' · '}
            {listing.connectedCount > 0
              ? `${listing.connectedCount} connected`
              : 'no connections yet'}
          </div>
        </div>
        <button
          onClick={onClose}
          aria-label="Close details"
          className="text-gray-600 hover:text-gray-300 transition-colors"
        >
          <X size={14} />
        </button>
      </div>

      {/* An entry from a catalog published before these fields existed knows
          nothing, which is different from knowing there is nothing. */}
      {!details.known ? (
        <p className="text-[12px] text-gray-500 mt-4">
          This connector does not describe itself yet. Add it to see what it offers.
        </p>
      ) : (
        <>
          <Section label="Starts a workflow when">
            {details.triggers.length > 0 ? (
              details.triggers.map((trigger) => (
                <Line key={trigger.type} tone="trigger" label={trigger.label}>
                  {trigger.description}
                </Line>
              ))
            ) : (
              <p className="text-[12px] text-gray-600">
                Nothing — this one is only called from a step.
              </p>
            )}
          </Section>

          <Section label="A step can ask it to">
            {details.actions.length > 0 ? (
              details.actions.map((action) => (
                <Line key={action.type} tone="action" label={action.label}>
                  {action.description}
                </Line>
              ))
            ) : (
              <p className="text-[12px] text-gray-600">Nothing — this one only watches.</p>
            )}
          </Section>

          {details.settings.length > 0 && (
            <Section label="You will need">
              <div className="flex flex-wrap gap-1.5">
                {required.map((setting) => (
                  <Setting key={setting.name} name={setting.name} required />
                ))}
                {optional.map((setting) => (
                  <Setting key={setting.name} name={setting.name} required={false} />
                ))}
              </div>
              {optional.length > 0 && (
                <p className="text-[10.5px] text-gray-600 mt-1.5">Dashed settings are optional.</p>
              )}
            </Section>
          )}
        </>
      )}

      {entry?.auth && (
        <div className="flex items-start gap-2 mt-3 text-[12px] text-gray-400">
          <KeyRound size={12} className="mt-0.5 shrink-0 text-gray-600" />
          <span>{entry.auth}</span>
        </div>
      )}

      <div className="flex items-center gap-2 mt-4">
        {/* An installed row has no manifest and no package spec, so there is
            nothing to open a form against. */}
        {listing.source !== 'installed' && (
          <button
            onClick={onAdd}
            className="text-xs text-gray-200 bg-white/[0.1] hover:bg-white/[0.16] px-3 py-1.5 rounded-sm transition-colors flex items-center gap-1"
          >
            <Plus size={12} /> Add connection
          </button>
        )}
        {entry?.packageName && (
          <a
            href={`https://www.npmjs.com/package/${entry.packageName}`}
            target="_blank"
            rel="noreferrer"
            className="text-xs text-gray-500 hover:text-gray-300 px-2 py-1.5 transition-colors flex items-center gap-1"
          >
            <ExternalLink size={11} /> View on npm
          </a>
        )}
        {listing.source === 'catalog' && (
          <span className="text-[10.5px] text-gray-600 ml-auto">
            Runs on demand. Nothing is installed until you add it.
          </span>
        )}
      </div>
    </div>
  )
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mt-4">
      <p className="text-[10px] text-gray-600 uppercase tracking-wider">{label}</p>
      <div className="mt-1.5 space-y-1.5">{children}</div>
    </div>
  )
}

function Line({
  tone,
  label,
  children
}: {
  tone: 'trigger' | 'action'
  label: string
  children?: React.ReactNode
}) {
  return (
    <div className="flex items-baseline gap-2 text-[12px]">
      <span
        aria-hidden
        className={`w-1 h-1 rounded-full shrink-0 ${
          tone === 'trigger' ? 'bg-cyan-400' : 'bg-amber-400'
        }`}
      />
      <span>
        <span className="text-gray-300">{label}</span>
        {children && <span className="text-gray-600"> — {children}</span>}
      </span>
    </div>
  )
}

function Setting({ name, required }: { name: string; required: boolean }) {
  return (
    <span
      className={`font-mono text-[10.5px] px-1.5 py-0.5 rounded-sm border ${
        required
          ? 'text-gray-400 border-white/[0.1]'
          : 'text-gray-600 border-white/[0.1] border-dashed'
      }`}
    >
      {name}
    </span>
  )
}
