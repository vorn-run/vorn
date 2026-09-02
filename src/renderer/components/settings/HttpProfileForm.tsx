import { useState } from 'react'
import { Check, AlertCircle } from 'lucide-react'

/**
 * An HTTP auth profile, made where it was asked for.
 *
 * The full connector form asks for six fields against a manifest; a template
 * that names a profile knows what it wants, so this asks for the three that
 * cannot be guessed and tests the result. Test runs after the profile exists,
 * because a preflight is against a connection id.
 */
export function HttpProfileForm({
  name,
  onDone,
  onCancel
}: {
  /** What the template called the profile, so the requirement rebinds to it. */
  name: string
  onDone: () => void
  onCancel: () => void
}) {
  const [profileName, setProfileName] = useState(name)
  const [baseUrl, setBaseUrl] = useState('')
  const [authHeader, setAuthHeader] = useState('Authorization: Bearer {{secret}}')
  const [secret, setSecret] = useState('')
  const [saving, setSaving] = useState(false)
  const [tested, setTested] = useState<{ ok: boolean | null; message?: string } | null>(null)
  const [error, setError] = useState<string | null>(null)

  const save = async (): Promise<void> => {
    setSaving(true)
    setError(null)
    setTested(null)
    try {
      const filters: Record<string, unknown> = { profileName, baseUrl, authHeader }
      // Encrypted here rather than stored: the same treatment every password
      // field in the connector forms gets.
      if (secret) filters.secret = await window.api.encryptString(secret)
      const connection = await window.api.createConnection({
        connectorId: 'http',
        name: profileName.trim() || name,
        filters,
        syncIntervalMinutes: 5,
        statusMapping: {}
      })
      const result = await window.api.preflightConnection(connection.id)
      setTested(result)
      // A profile that answers is done with; one that does not stays open so
      // the base URL or the secret can be corrected against what it said.
      if (result.ok !== false) onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'The profile could not be saved')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="px-3 py-2.5 bg-white/[0.03] border border-white/[0.08] rounded-sm">
      <h3 className="text-[12px] font-medium text-gray-200 mb-2">HTTP profile</h3>

      <Field label="Name" value={profileName} onChange={setProfileName} />
      <Field
        label="Base URL"
        value={baseUrl}
        onChange={setBaseUrl}
        placeholder="https://api.example.com"
      />
      <Field label="Auth header" value={authHeader} onChange={setAuthHeader} mono />
      <Field label="Secret" value={secret} onChange={setSecret} type="password" />

      {tested && (
        <p
          className={`flex items-start gap-1.5 text-[11px] mt-2 ${
            tested.ok === false ? 'text-danger' : 'text-status-sage'
          }`}
        >
          {tested.ok === false ? (
            <AlertCircle size={12} className="mt-0.5 shrink-0" />
          ) : (
            <Check size={12} className="mt-0.5 shrink-0" />
          )}
          {tested.message ?? (tested.ok ? 'The profile answered.' : 'No answer to check.')}
        </p>
      )}
      {error && <p className="text-[11px] text-danger mt-2">{error}</p>}

      <div className="flex justify-end gap-2 mt-3">
        <button
          onClick={onCancel}
          className="text-[11px] text-gray-400 hover:text-white px-2.5 py-1 rounded-sm
                     hover:bg-white/[0.06] transition-colors"
        >
          Cancel
        </button>
        <button
          onClick={() => void save()}
          disabled={saving || profileName.trim() === ''}
          className="text-[11px] text-gray-200 hover:text-white px-2.5 py-1 border
                     border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors
                     disabled:opacity-50"
        >
          {saving ? 'Testing…' : 'Save and test'}
        </button>
      </div>
    </div>
  )
}

function Field({
  label,
  value,
  onChange,
  placeholder,
  type = 'text',
  mono
}: {
  label: string
  value: string
  onChange: (value: string) => void
  placeholder?: string
  type?: string
  mono?: boolean
}) {
  return (
    <label className="block mb-2">
      <span className="block text-[11px] text-gray-500 mb-1">{label}</span>
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full px-2 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm
                    text-[12px] text-gray-200 outline-none focus:border-white/[0.2]
                    ${mono ? 'font-mono' : ''}`}
      />
    </label>
  )
}
