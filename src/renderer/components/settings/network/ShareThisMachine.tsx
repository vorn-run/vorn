import { useState } from 'react'
import type { TailscaleStatus, ReachableUrls } from '../../../../shared/types'
import { SettingRow } from '../SettingRow'
import { ToggleSwitch } from '../ToggleSwitch'
import { CopyButton, QRCode128 } from './shared'
import { DeviceTokenList } from './DeviceTokenList'

/**
 * Where this Vorn can be reached, led by one address.
 *
 * A machine usually answers on several, and only the person looking at the screen
 * knows which network the other device is on, so all of them stay available. But
 * they are not equally useful: the server returns them tailnet first, and that one
 * is encrypted. Showing three identical boxes made the reader choose before anything
 * had told them how to.
 *
 * The qualifier under the address is the honest difference between them, and it
 * replaces the Tailscale card that used to sit further down explaining the same
 * thing at more length.
 */
function AddressBlock({
  reachable,
  onTailnet,
  tailscaleInstalled
}: {
  reachable: ReachableUrls | null
  onTailnet: boolean
  tailscaleInstalled: boolean
}) {
  const [showAll, setShowAll] = useState(false)

  if (!reachable || reachable.urls.length === 0) return null
  const [primary, ...rest] = reachable.urls

  return (
    <div className="mt-3 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium mb-1.5">
            Open on your phone
          </div>
          <div className="flex items-center">
            <code className="text-sm text-emerald-400 font-mono truncate">{primary}</code>
            <CopyButton text={primary} />
          </div>
          <p className="text-xs text-gray-600 mt-2">
            {onTailnet ? (
              <>Encrypted via Tailscale</>
            ) : (
              <>
                This network only, unencrypted
                {!tailscaleInstalled && (
                  <>
                    {' · '}
                    <a
                      href="https://tailscale.com/download"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-gray-400 hover:text-white transition-colors"
                    >
                      Add Tailscale
                    </a>
                  </>
                )}
              </>
            )}
            {rest.length > 0 && (
              <>
                {' · '}
                <button
                  onClick={() => setShowAll(!showAll)}
                  className="text-gray-400 hover:text-white transition-colors"
                >
                  {showAll ? 'Hide' : `${rest.length} other address${rest.length > 1 ? 'es' : ''}`}
                </button>
              </>
            )}
          </p>
        </div>
        <QRCode128 url={primary} />
      </div>

      {showAll && rest.length > 0 && (
        <div className="mt-3 pt-3 border-t border-white/[0.06]">
          {rest.map((url) => (
            <div key={url} className="flex items-center py-1">
              <code className="text-xs text-gray-400 font-mono flex-1 truncate">{url}</code>
              <CopyButton text={url} />
            </div>
          ))}
          <p className="text-[10px] text-gray-600 mt-1.5">
            Use whichever address the other device shares a network with.
          </p>
        </div>
      )}
    </div>
  )
}

/**
 * Share this machine's server with other devices.
 *
 * One of the two directions Remote Access can point. The other is
 * `UseAnotherMachine`, and they are mutually exclusive: host mode never starts a
 * local server, so there would be nothing here to share.
 */
export function ShareThisMachine({
  enabled,
  status,
  reachable,
  onChange
}: {
  enabled: boolean
  status: TailscaleStatus | null
  reachable: ReachableUrls | null
  onChange: (value: boolean) => void
}) {
  /** Set while the toggle is on its way up without a tailnet. See the panel below. */
  const [confirming, setConfirming] = useState(false)
  const onTailnet = status?.running === true

  const apply = (value: boolean): void => {
    setConfirming(false)
    onChange(value)
  }

  return (
    <div>
      <SettingRow label="Share this machine" description="Let other devices open Vorn in a browser">
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => {
            // Turning it off is never worth a confirmation, and neither is turning it
            // on over a tailnet, where the traffic is encrypted either way.
            if (!value || onTailnet) return apply(value)
            setConfirming(true)
          }}
        />
      </SettingRow>

      {/* Naming what the wide bind costs is the whole reason this panel has any
          weight at all. Deliberately heavier than everything around it. */}
      {confirming && (
        <div className="mt-3 rounded-lg border border-amber-500/20 bg-amber-500/[0.04] p-4">
          <div className="text-sm font-medium text-amber-200 mb-1">Enable without Tailscale?</div>
          <div className="text-xs text-gray-400 space-y-1.5">
            <p>
              Vorn will accept connections from anything on this network, over plain HTTP. A device
              token is still required, but it travels unencrypted, so anyone who can read this
              network&apos;s traffic can take it. A token runs commands on this machine.
            </p>
            <p>Fine on a home or office network you trust. Not on shared or public wifi.</p>
          </div>
          <div className="flex gap-2 mt-3">
            <button
              onClick={() => apply(true)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Enable anyway
            </button>
            <button
              onClick={() => setConfirming(false)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {enabled && (
        <>
          <AddressBlock
            reachable={reachable}
            onTailnet={onTailnet}
            tailscaleInstalled={status?.installed === true}
          />
          <DeviceTokenList />
        </>
      )}
    </div>
  )
}
