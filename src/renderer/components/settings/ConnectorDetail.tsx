import { Plus, ArrowLeft, ExternalLink } from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import {
  listingDetails,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'

/**
 * What a connector does, before anything is downloaded.
 *
 * Replaces the list rather than appearing beneath it, the way both catalogs
 * worth copying do: nothing reflows, nothing has to fit beside anything else,
 * and it reads the same at any width.
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

  return (
    <div>
      <button
        onClick={onClose}
        className="flex items-center gap-1.5 text-[12px] text-gray-500 hover:text-gray-300 transition-colors mb-4"
      >
        <ArrowLeft size={12} /> All connectors
      </button>

      <div className="flex items-start gap-3">
        <span className="w-10 h-10 shrink-0 flex items-center justify-center bg-white/[0.05] rounded-md">
          <ConnectorIcon
            connectorId={listing.id}
            icon={entry?.icon ?? listing.icon}
            size={21}
            className="text-gray-200"
          />
        </span>
        <div className="min-w-0">
          <div className="text-[16px] text-gray-100 font-medium tracking-tight">{listing.name}</div>
          <div className="text-[11.5px] text-gray-600">
            {entry?.packageName ?? (listing.source === 'builtin' ? 'Built in' : listing.id)}
            {entry?.version && ` · v${entry.version}`}
            {listing.connectedCount > 0 && ` · ${count(listing.connectedCount, 'connection')}`}
          </div>
        </div>
      </div>

      {listing.description && (
        <p className="text-[12.5px] text-gray-400 mt-4 leading-relaxed">{listing.description}</p>
      )}

      {/* An entry from a catalog published before these fields existed knows
          nothing, which is different from knowing there is nothing. */}
      {!details.known ? (
        <p className="text-[12px] text-gray-500 mt-5">
          This connector does not describe itself yet. Add it to see what it offers.
        </p>
      ) : (
        <>
          <Section label="Starts a workflow when">
            {details.triggers.length > 0 ? (
              details.triggers.map((trigger) => (
                <Item key={trigger.type} title={trigger.label} detail={trigger.description} />
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
                <Item key={action.type} title={action.label} detail={action.description} />
              ))
            ) : (
              <p className="text-[12px] text-gray-600">Nothing — this one only watches.</p>
            )}
          </Section>

          {details.settings.length > 0 && (
            <Section label="You will need">
              <p className="font-mono text-[11px] text-gray-400 leading-relaxed">
                {details.settings.map((setting, index) => (
                  <span key={setting.name}>
                    {index > 0 && '   '}
                    <span className={setting.required ? undefined : 'text-gray-600'}>
                      {setting.name}
                      {!setting.required && ' (optional)'}
                    </span>
                  </span>
                ))}
              </p>
            </Section>
          )}
        </>
      )}

      {entry?.auth && <p className="text-[12.5px] text-gray-400 mt-4">{entry.auth}</p>}

      <div className="flex items-center gap-2 mt-6">
        {/* An installed row has no manifest and no package spec, so there is
            nothing to open a form against. */}
        {listing.source !== 'installed' && (
          <button
            onClick={onAdd}
            className="text-xs text-gray-100 bg-white/[0.1] hover:bg-white/[0.16] px-3 py-1.5 rounded-sm transition-colors flex items-center gap-1"
          >
            <Plus size={12} /> Add a connection
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
          <span className="text-[11px] text-gray-600 ml-auto">
            Runs on demand. Nothing is installed until you add it.
          </span>
        )}
      </div>
    </div>
  )
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mt-5">
      <p className="text-[10px] text-gray-600 uppercase tracking-wider">{label}</p>
      <div className="mt-2 space-y-2">{children}</div>
    </div>
  )
}

function Item({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="text-[12.5px]">
      <div className="text-gray-300">{title}</div>
      {detail && <div className="text-gray-600 mt-0.5">{detail}</div>}
    </div>
  )
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
