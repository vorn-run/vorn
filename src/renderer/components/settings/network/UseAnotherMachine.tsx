import { useState } from 'react'

/**
 * Point this desktop at a Vorn running somewhere else.
 *
 * The other direction Remote Access can point. It is exclusive with
 * `ShareThisMachine` rather than merely separate from it: connecting to a host skips
 * the local spawn entirely, because two servers would mean two databases and the
 * local one would shadow the remote without saying so. There is nothing to share in
 * this mode, which is why the sharing controls are not merely disabled here but
 * absent.
 *
 * Applying restarts the app. The spawn-or-connect decision is made once at startup
 * and everything downstream assumes its answer.
 */
export function UseAnotherMachine({ mode, url: currentUrl }: { mode: string; url: string }) {
  const connected = mode === 'host'
  const [url, setUrl] = useState(currentUrl)
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [editing, setEditing] = useState(!connected)

  const apply = async (): Promise<void> => {
    setError(null)
    const result = await window.api.saveConnectSettings({ url, token })
    if (!result.ok) setError(result.error ?? 'Could not save those details.')
  }

  if (connected && !editing) {
    return (
      <div>
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm text-gray-200">
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 shrink-0" />
                Connected
              </div>
              <code className="block text-xs text-gray-600 font-mono mt-1.5 truncate">
                {currentUrl}
              </code>
            </div>
            <div className="flex gap-2 shrink-0">
              <button
                onClick={() => setEditing(true)}
                className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-300 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
              >
                Change
              </button>
              <button
                onClick={() => void window.api.useLocalServer()}
                className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
              >
                Disconnect
              </button>
            </div>
          </div>
        </div>
        <p className="text-xs text-gray-600 mt-3">
          Sessions, tasks and projects live on that machine. Workflows and schedules run only while
          a desktop is attached to it.
        </p>
      </div>
    )
  }

  return (
    <div>
      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
        <label className="block text-xs text-gray-400 mb-1.5" htmlFor="host-url">
          Server address
        </label>
        <input
          id="host-url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="192.168.0.4:61601"
          spellCheck={false}
          className="w-full rounded-md bg-white/[0.04] border border-white/[0.08] px-3 py-2 text-sm text-gray-200 font-mono placeholder:text-gray-600 outline-none focus:border-white/20"
        />
        <label className="block text-xs text-gray-400 mb-1.5 mt-3" htmlFor="host-token">
          Device token from that machine
        </label>
        <input
          id="host-token"
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="vorn_..."
          spellCheck={false}
          className="w-full rounded-md bg-white/[0.04] border border-white/[0.08] px-3 py-2 text-sm text-gray-200 font-mono placeholder:text-gray-600 outline-none focus:border-white/20"
        />

        {error && <p className="text-xs text-amber-400/80 mt-2">{error}</p>}

        <div className="flex gap-2 mt-3">
          <button
            onClick={() => void apply()}
            className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
          >
            Connect and restart
          </button>
          {connected && (
            <button
              onClick={() => {
                setEditing(false)
                setError(null)
                setToken('')
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          )}
        </div>
      </div>
      <p className="text-xs text-gray-600 mt-3">
        Vorn restarts to apply this. Workflows and schedules run only while a desktop is attached to
        the host, so a server on its own holds your data but fires no schedules.
      </p>
    </div>
  )
}
