import { useEffect, useRef } from 'react'
import { borrowableFromManifest, type ConnectorPackSummary } from '../../../shared/types'
import { ConnectorIcon } from '../ConnectorIcon'

/**
 * What a pack is and what it can do, before any of it is kept.
 *
 * A dropped file used to install on the strength of the drop alone; the same
 * verification now answers a question first, so a connector states its
 * triggers and actions while refusing is still one click.
 */
export function PackInstallConfirm({
  preview,
  busy,
  onConfirm,
  onCancel
}: {
  preview: ConnectorPackSummary
  busy?: boolean
  onConfirm: () => void
  onCancel: () => void
}) {
  const replacing = preview.installedVersion && preview.installedVersion !== preview.version
  const required = (preview.env ?? []).filter((entry) => entry.required)
  const borrows = borrowableFromManifest(preview.auth, preview.env ?? [])
  const root = useRef<HTMLDivElement>(null)

  // It opens beside the button that raised it, which can be below the fold.
  useEffect(() => {
    root.current?.scrollIntoView?.({ block: 'nearest' })
  }, [])

  return (
    <div ref={root} className="border border-white/[0.1] rounded-sm bg-white/[0.02] p-3">
      <div className="flex items-start gap-2.5">
        <ConnectorIcon
          connectorId={preview.id}
          {...(preview.icon && { icon: preview.icon })}
          size={20}
          className="text-gray-300 mt-0.5"
        />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] text-gray-200 font-medium">
            {preview.name} <span className="text-gray-500 font-normal">v{preview.version}</span>
          </div>
          {preview.description && (
            <p className="text-[11px] text-gray-500 mt-0.5">{preview.description}</p>
          )}
          {replacing && (
            <p className="text-[11px] text-gray-400 mt-1">
              Replaces v{preview.installedVersion}, which is kept so you can roll back.
            </p>
          )}
        </div>
      </div>

      <dl className="mt-3 space-y-1.5">
        <PackCapability
          label="Triggers"
          items={preview.triggers.map((trigger) => trigger.label || trigger.type)}
        />
        <PackCapability
          label="Actions"
          items={preview.actions.map((action) => action.label || action.type)}
        />
        {required.length > 0 && (
          <PackCapability label="Needs" items={required.map((entry) => entry.name)} />
        )}
        {borrows.length > 0 && (
          <PackCapability
            label="Borrows"
            items={borrows.map(
              (name) => `${name} from ${preview.auth?.probe?.command ?? 'a signed-in tool'}`
            )}
          />
        )}
      </dl>

      <div className="flex justify-end gap-2 mt-3">
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          className="px-2.5 py-1 text-[11px] text-gray-400 hover:text-gray-200 border border-white/[0.1] rounded-sm disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={onConfirm}
          disabled={busy}
          className="px-2.5 py-1 text-[11px] text-gray-200 hover:text-white border border-white/[0.15] rounded-sm disabled:opacity-50"
        >
          {busy ? 'Installing…' : replacing ? 'Update' : 'Install'}
        </button>
      </div>
    </div>
  )
}

/** One capability row; silent rather than empty when a connector has none of a kind. */
function PackCapability({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null
  return (
    <div className="flex gap-2 text-[11px]">
      <dt className="text-gray-600 w-14 shrink-0">{label}</dt>
      <dd className="text-gray-400 flex-1">{items.join(', ')}</dd>
    </div>
  )
}
