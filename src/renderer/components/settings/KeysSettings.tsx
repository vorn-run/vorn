import { useCallback, useEffect, useState } from 'react'
import { KeyRound, RefreshCw } from 'lucide-react'
import type { ConnectorKey, ConnectorKeyField } from '../../../shared/types'
import { ConnectorIcon } from '../ConnectorIcon'

/** What a key's stored value can be said to be, without saying what it is. */
function describeField(field: ConnectorKeyField): string {
  if (!field.readable) return 'Locked — this machine cannot read it'
  if (field.envNames) {
    return field.envNames.length > 0 ? field.envNames.join(' · ') : 'No variables set'
  }
  return field.hint || 'Set'
}

/** "used by 2 steps" is what says what rotating this is about to touch. */
function describeUsage(count: number): string {
  if (count === 0) return 'unused'
  return count === 1 ? 'used by 1 step' : `used by ${count} steps`
}

function KeyRow({ entry, onRotated }: { entry: ConnectorKey; onRotated: () => void }) {
  const [testing, setTesting] = useState(false)
  const [result, setResult] = useState<{ ok: boolean | null; message?: string } | null>(null)
  const [rotating, setRotating] = useState<string | null>(null)
  const [value, setValue] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const runTest = async () => {
    setTesting(true)
    setResult(null)
    try {
      setResult(await window.api.preflightConnection(entry.connectionId))
    } catch (err) {
      setResult({ ok: false, message: err instanceof Error ? err.message : String(err) })
    } finally {
      setTesting(false)
    }
  }

  const save = async (field: string) => {
    setSaving(true)
    setError(null)
    try {
      // Encrypted here: the keychain that can seal it lives in this process,
      // and the server is only ever handed ciphertext.
      const sealed = await window.api.encryptString(value)
      const outcome = await window.api.rotateConnectionSecret({
        connectionId: entry.connectionId,
        field,
        value: sealed
      })
      if (!outcome.ok) {
        setError(outcome.error ?? 'The key could not be replaced')
        return
      }
      setRotating(null)
      setValue('')
      setResult(null)
      onRotated()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="px-4 py-2.5 bg-white/[0.03] border border-white/[0.06] rounded-sm">
      <div className="flex items-center gap-3">
        <ConnectorIcon connectorId={entry.connectorId} size={14} className="text-gray-400" />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] text-gray-200 truncate">{entry.name}</div>
          <div className="text-[11px] text-gray-500 truncate">
            {entry.fields.map(describeField).join(' · ')} · {describeUsage(entry.usageCount)}
          </div>
        </div>
        {result && (
          <span
            aria-hidden
            className={`w-1.5 h-1.5 rounded-full shrink-0 ${
              result.ok === null ? 'bg-gray-600' : result.ok ? 'bg-status-sage' : 'bg-danger'
            }`}
          />
        )}
        <button
          onClick={runTest}
          disabled={testing}
          className="text-[11px] text-gray-400 hover:text-gray-200 px-2 py-1 border border-white/[0.1] rounded-sm disabled:opacity-50"
        >
          {testing ? 'Testing…' : 'Test'}
        </button>
        {entry.fields.map((field) => (
          <button
            key={field.key}
            onClick={() => setRotating(rotating === field.key ? null : field.key)}
            className="text-[11px] text-gray-400 hover:text-gray-200 px-2 py-1 border border-white/[0.1] rounded-sm flex items-center gap-1"
          >
            <RefreshCw size={11} />
            {entry.fields.length > 1 ? `Rotate ${field.label}` : 'Rotate'}
          </button>
        ))}
      </div>

      {result && (
        <div
          className={`mt-1.5 text-[11px] ${result.ok === false ? 'text-danger' : 'text-gray-500'}`}
        >
          {result.ok === null
            ? 'This connector has nothing to check'
            : (result.message ?? (result.ok ? 'Reached it' : 'Could not reach it'))}
        </div>
      )}

      {rotating && (
        <div className="mt-2 flex items-center gap-2">
          <input
            type="password"
            autoFocus
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={
              entry.fields.find((f) => f.key === rotating)?.envNames
                ? '{"TOKEN": "…"}'
                : 'The replacement value'
            }
            className="flex-1 px-2 py-1 bg-white/[0.05] border border-white/[0.1] rounded-sm text-xs text-gray-200 outline-none focus:border-white/[0.2]"
          />
          <button
            onClick={() => save(rotating)}
            disabled={saving || value === ''}
            className="text-[11px] text-gray-200 px-2.5 py-1 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] disabled:opacity-50"
          >
            {saving ? 'Saving…' : 'Save'}
          </button>
          <button
            onClick={() => {
              setRotating(null)
              setValue('')
              setError(null)
            }}
            className="text-[11px] text-gray-500 hover:text-gray-300 px-2 py-1"
          >
            Cancel
          </button>
        </div>
      )}
      {error && <div className="mt-1.5 text-[11px] text-danger">{error}</div>}
    </div>
  )
}

export function KeysSettings() {
  const [keys, setKeys] = useState<ConnectorKey[]>([])
  const [safeStorageAvailable, setSafeStorageAvailable] = useState(true)
  const [loaded, setLoaded] = useState(false)

  const load = useCallback(async () => {
    try {
      setKeys((await window.api.listConnectorKeys?.()) ?? [])
    } catch {
      setKeys([])
    } finally {
      setLoaded(true)
    }
  }, [])

  useEffect(() => {
    void load()
    let cancelled = false
    window.api
      .isSafeStorageAvailable?.()
      .then((available) => {
        if (!cancelled) setSafeStorageAvailable(available)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [load])

  return (
    <div data-testid="keys-panel">
      <h2 className="text-xl font-semibold text-white mb-1">Keys</h2>
      <p className="text-sm text-gray-500 mb-6">
        The secrets your connections hold, stored encrypted with your OS keychain
      </p>

      {!safeStorageAvailable && (
        <div
          className="mb-4 px-4 py-3 rounded-sm border border-amber-500/30 text-sm text-amber-400"
          style={{ background: 'rgba(245, 158, 11, 0.08)' }}
        >
          Keychain encryption is not available on this system, so a key cannot be replaced from
          here.
        </div>
      )}

      <div className="space-y-2">
        {keys.map((entry) => (
          <KeyRow key={entry.connectionId} entry={entry} onRotated={load} />
        ))}
      </div>

      {loaded && keys.length === 0 && (
        <div className="px-4 py-6 border border-white/[0.06] rounded-sm text-center">
          <KeyRound size={16} className="mx-auto text-gray-600 mb-2" />
          <div className="text-sm text-gray-400">No keys yet</div>
          <div className="text-[11px] text-gray-600 mt-1">
            A connection that signs in with a key or a token shows up here.
          </div>
        </div>
      )}
    </div>
  )
}
