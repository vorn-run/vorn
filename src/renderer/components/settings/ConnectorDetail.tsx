import {
  Plus,
  ArrowLeft,
  Check,
  ExternalLink,
  Download,
  RefreshCw,
  Undo2,
  Trash2,
  Workflow
} from 'lucide-react'
import { ConnectorIcon } from '../ConnectorIcon'
import type { ConnectorCatalogVerification, ConnectorInstallProgress } from '../../../shared/types'
import {
  listingDetails,
  AUTH_RUNG,
  type BuiltInConnector,
  type ConnectorListing
} from '../../lib/connector-browse'
import { canAddConnection, describePackStatus, packStateFor } from '../../lib/pack-status'
import type { RowState } from '../../lib/use-row-action'
import { TONE_DOT, TONE_TEXT } from '../../lib/status-tone'
import { ActivityLine } from './ActivityLine'
import { BusyIcon } from './BusyIcon'

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
  progress,
  activity,
  pending,
  onAdd,
  onInstall,
  onRollback,
  onRemove,
  onUse,
  onClose
}: {
  listing: ConnectorListing
  builtIns: BuiltInConnector[]
  /** The install running for this connector, when one is. */
  progress?: ConnectorInstallProgress
  /** What this connector's own actions are doing, and what the last one answered. */
  activity?: RowState
  /** The confirm sheet for this connector's pack, once it has been verified. */
  pending?: React.ReactNode
  onAdd: () => void
  onInstall?: () => void
  onRollback?: () => void
  onRemove?: () => void
  /** Where a connector that needs no connection is actually used. */
  onUse?: () => void
  onClose: () => void
}) {
  const details = listingDetails(listing, builtIns)
  const entry = listing.catalogItem
  const state = packStateFor({
    installed: listing.pack,
    catalogVersion: entry?.version,
    progress
  })
  const status = describePackStatus(state)
  const busy = Boolean(activity?.phrase)

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
          {/* A badge is a claim; this is the claim's receipt, said in full. */}
          {listing.verified && <VerifiedStrip verification={listing.verified} />}
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

      {/* The rung answers "what will this ask of me"; the connector's own sentence says it in its terms. */}
      {(listing.authRung || entry?.auth) && (
        <Section label="Signs in with">
          {listing.authRung && (
            <p className="text-[12.5px] text-gray-300">{AUTH_RUNG[listing.authRung].detail}</p>
          )}
          {entry?.auth && <p className="text-[12.5px] text-gray-500">{entry.auth}</p>}
        </Section>
      )}

      {/* The question this page could not answer while a connector was a package name. */}
      {listing.pack && (
        <Section label="On this machine">
          <dl className="text-[12px] leading-relaxed">
            <Fact term="Installed" value={`v${listing.pack.version}`} />
            <Fact term="On disk" value={listing.pack.path} mono />
            <Fact term="Runs via" value={`node ${listing.pack.path}/index.js`} mono />
          </dl>
        </Section>
      )}

      {status.detail && state.kind !== 'installed' && (
        <p className={`flex items-start gap-1.5 text-[12px] mt-4 ${TONE_TEXT[status.tone]}`}>
          <span className={`w-1.5 h-1.5 rounded-full shrink-0 mt-1.5 ${TONE_DOT[status.tone]}`} />
          {status.detail}
        </p>
      )}

      <ActivityLine {...activity} className="text-[12px] mt-4" />

      <div className="flex items-center gap-2 mt-6">
        {/* Nothing to connect: installing it was the whole setup, so the next move is the one it exists for. */}
        {listing.implicitlyConnected ? (
          onUse ? (
            <button
              onClick={onUse}
              className="text-xs text-gray-100 bg-white/[0.1] hover:bg-white/[0.16] px-3 py-1.5 rounded-sm transition-colors flex items-center gap-1"
            >
              <Workflow size={12} /> Use in a workflow
            </button>
          ) : (
            <span className="text-[12px] text-gray-500">Ready to use in a workflow.</span>
          )
        ) : (
          canAddConnection(state, {
            source: listing.source,
            hasLegacyLaunch: Boolean(entry?.packageName)
          }) && (
            <button
              onClick={onAdd}
              className="text-xs text-gray-100 bg-white/[0.1] hover:bg-white/[0.16] px-3 py-1.5 rounded-sm transition-colors flex items-center gap-1"
            >
              <Plus size={12} /> Add a connection
            </button>
          )
        )}

        {onInstall && status.action && (
          <button
            onClick={onInstall}
            disabled={status.busy}
            title={
              state.kind === 'absent'
                ? 'Install this connector as a file'
                : 'Install the newer version'
            }
            className="text-xs text-gray-300 hover:text-white px-2.5 py-1.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
          >
            {status.action === 'install' ? <Download size={12} /> : <RefreshCw size={12} />}
            {status.label}
          </button>
        )}

        {onRollback && state.kind === 'installed' && state.previousVersion && (
          <button
            onClick={onRollback}
            disabled={busy}
            title={`Go back to v${state.previousVersion}`}
            className="text-xs text-gray-400 hover:text-gray-200 px-2.5 py-1.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
          >
            <BusyIcon busy={busy} icon={Undo2} size={12} /> Roll back
          </button>
        )}

        {onRemove && listing.pack && (
          <button
            onClick={onRemove}
            disabled={busy}
            title="Delete the installed files"
            className="text-xs text-danger hover:text-danger px-2.5 py-1.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors flex items-center gap-1 disabled:opacity-50"
          >
            <BusyIcon busy={busy} icon={Trash2} size={12} /> Remove
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

        {listing.source === 'catalog' && !listing.pack && (
          <span className="text-[11px] text-gray-600 ml-auto">
            Nothing is on disk until you install it.
          </span>
        )}
      </div>

      {/* Under the button that raised it, where the answer is looked for. */}
      {pending && <div className="mt-3">{pending}</div>}
    </div>
  )
}

/**
 * The receipt behind the badge: what ran, against which version, and when.
 *
 * Named checks rather than a score, because "verified" is only worth anything
 * if it says what was verified — and a date, because a check that ran against
 * a version three releases back vouches for less than it appears to.
 */
function VerifiedStrip({ verification }: { verification: ConnectorCatalogVerification }) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 mt-1.5 text-[11px] text-gray-500">
      {verification.checks.map((check) => (
        <span key={check} className="inline-flex items-center gap-1">
          <Check size={10} /> {check}
        </span>
      ))}
      <span className="text-gray-600">
        checked {formatDay(verification.checkedAt)} against v{verification.version}
      </span>
    </div>
  )
}

/** A date said the way a person would, falling back to the raw stamp. */
function formatDay(iso: string): string {
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return iso
  return at.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

function Fact({ term, value, mono }: { term: string; value: string; mono?: boolean }) {
  return (
    <div className="flex gap-3">
      <dt className="text-gray-600 w-[72px] shrink-0">{term}</dt>
      <dd className={`text-gray-300 min-w-0 break-all ${mono ? 'font-mono text-[11px]' : ''}`}>
        {value}
      </dd>
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
