import { useState, useEffect } from 'react'
import { useAppStore } from '../../stores'
import { RemoteHost, AuthMethod, SSHKeyMeta } from '../../../shared/types'
import { Server, Trash2, Plus, Key, Loader2, CheckCircle2, XCircle, Wifi } from 'lucide-react'

const AUTH_METHODS: { value: AuthMethod; label: string }[] = [
  { value: 'agent', label: 'SSH Agent' },
  { value: 'password', label: 'Password' },
  { value: 'key-stored', label: 'Stored Key' },
  { value: 'key-file', label: 'Key File' }
]

export function HostSettings() {
  const config = useAppStore((s) => s.config)
  const addRemoteHost = useAppStore((s) => s.addRemoteHost)
  const removeRemoteHost = useAppStore((s) => s.removeRemoteHost)
  const updateRemoteHost = useAppStore((s) => s.updateRemoteHost)

  const [drafts, setDrafts] = useState<Record<string, Partial<RemoteHost>>>({})
  const [storedKeys, setStoredKeys] = useState<SSHKeyMeta[]>([])
  const [passwordInputs, setPasswordInputs] = useState<Record<string, string>>({})
  const [testResults, setTestResults] = useState<
    Record<string, { status: 'testing' | 'success' | 'error'; message?: string }>
  >({})

  useEffect(() => {
    let cancelled = false
    window.api
      .listSSHKeys()
      .then((keys) => {
        if (!cancelled) setStoredKeys(keys)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  if (!config) return null

  const hosts = config.remoteHosts || []

  const getDraft = (host: RemoteHost): RemoteHost => ({
    ...host,
    ...drafts[host.id]
  })

  const updateDraft = (id: string, patch: Partial<RemoteHost>): void => {
    setDrafts((prev) => ({ ...prev, [id]: { ...prev[id], ...patch } }))
  }

  const commitDraft = (id: string): void => {
    const draft = drafts[id]
    if (!draft) return
    const host = hosts.find((h) => h.id === id)
    if (!host) return
    updateRemoteHost(id, { ...host, ...draft })
    setDrafts((prev) => {
      const next = { ...prev }
      delete next[id]
      return next
    })
  }

  const handleAddHost = (): void => {
    const id = crypto.randomUUID()
    addRemoteHost({
      id,
      label: 'New Host',
      hostname: '',
      user: '',
      port: 22,
      authMethod: 'agent'
    })
  }

  const handleBrowseKey = async (hostId: string): Promise<void> => {
    const filePath = await window.api.openFileDialog()
    if (filePath) {
      const host = hosts.find((h) => h.id === hostId)
      if (host) {
        updateRemoteHost(hostId, { ...host, ...drafts[hostId], sshKeyPath: filePath })
        setDrafts((prev) => {
          const next = { ...prev }
          delete next[hostId]
          return next
        })
      }
    }
  }

  const handleAuthMethodChange = (hostId: string, method: AuthMethod): void => {
    const host = hosts.find((h) => h.id === hostId)
    if (!host) return
    const updated: RemoteHost = {
      ...host,
      ...drafts[hostId],
      authMethod: method
    }
    // Clear irrelevant fields when switching
    if (method !== 'key-file') updated.sshKeyPath = undefined
    if (method !== 'key-stored') updated.credentialId = undefined
    if (method !== 'password') updated.encryptedPassword = undefined
    updateRemoteHost(hostId, updated)
    setDrafts((prev) => {
      const next = { ...prev }
      delete next[hostId]
      return next
    })
    // Clear plaintext password from memory when switching away
    if (method !== 'password') {
      setPasswordInputs((prev) => {
        const next = { ...prev }
        delete next[hostId]
        return next
      })
    }
  }

  const handlePasswordCommit = async (hostId: string): Promise<void> => {
    const pw = passwordInputs[hostId]
    if (!pw) return
    try {
      const encrypted = await window.api.encryptString(pw)
      const host = hosts.find((h) => h.id === hostId)
      if (host) {
        updateRemoteHost(hostId, { ...host, ...drafts[hostId], encryptedPassword: encrypted })
        setDrafts((prev) => {
          const next = { ...prev }
          delete next[hostId]
          return next
        })
        setPasswordInputs((prev) => {
          const next = { ...prev }
          delete next[hostId]
          return next
        })
      }
    } catch {
      /* encryption not available */
    }
  }

  const handleStoredKeyChange = (hostId: string, credentialId: string): void => {
    const host = hosts.find((h) => h.id === hostId)
    if (!host) return
    updateRemoteHost(hostId, { ...host, ...drafts[hostId], credentialId })
    setDrafts((prev) => {
      const next = { ...prev }
      delete next[hostId]
      return next
    })
  }

  const getSshPreview = (h: RemoteHost): string => {
    const parts = ['ssh']
    if (h.user) parts.push(`${h.user}@${h.hostname || '...'}`)
    else parts.push(h.hostname || '...')
    if (h.port !== 22) parts.push(`-p ${h.port}`)
    const auth = h.authMethod ?? 'agent'
    if (auth === 'key-file' && h.sshKeyPath) parts.push(`-i ${h.sshKeyPath}`)
    if (auth === 'password') parts.push('-o PreferredAuthentications=password')
    return parts.join(' ')
  }

  const handleTestConnection = async (hostId: string): Promise<void> => {
    const host = hosts.find((h) => h.id === hostId)
    if (!host) return
    const effective = { ...host, ...drafts[hostId] }
    if (!effective.hostname || !effective.user) {
      setTestResults((prev) => ({
        ...prev,
        [hostId]: { status: 'error', message: 'Hostname and username are required' }
      }))
      return
    }
    setTestResults((prev) => ({ ...prev, [hostId]: { status: 'testing' } }))
    try {
      const result = await window.api.testSshConnection(effective)
      setTestResults((prev) => ({
        ...prev,
        [hostId]: { status: result.success ? 'success' : 'error', message: result.message }
      }))
    } catch (err) {
      setTestResults((prev) => ({
        ...prev,
        [hostId]: { status: 'error', message: String(err) }
      }))
    }
  }

  return (
    <div>
      <h2 className="text-xl font-semibold text-white mb-1">Remote Hosts</h2>
      <p className="text-sm text-gray-500 mb-6">
        Configure SSH hosts to run coding agents on remote machines
      </p>

      <div className="space-y-3">
        {hosts.map((host) => {
          const h = getDraft(host)
          const authMethod = h.authMethod ?? 'agent'

          return (
            <div
              key={host.id}
              className="border border-white/[0.06] rounded-lg p-4"
              style={{ background: 'var(--color-surface-sunken)' }}
            >
              <div className="flex items-center gap-3 mb-4">
                <Server size={18} strokeWidth={1.5} className="text-blue-400 shrink-0" />
                <input
                  type="text"
                  value={h.label}
                  onChange={(e) => updateDraft(host.id, { label: e.target.value })}
                  onBlur={() => commitDraft(host.id)}
                  className="flex-1 text-sm font-medium text-gray-200 bg-transparent
                             border-b border-transparent hover:border-white/[0.1] focus:border-white/[0.2]
                             focus:outline-none py-0.5 px-1 -ml-1"
                  placeholder="Host label"
                />
                <button
                  onClick={() => removeRemoteHost(host.id)}
                  className="text-gray-600 hover:text-red-400 p-1 transition-colors"
                >
                  <Trash2 size={14} strokeWidth={1.5} />
                </button>
              </div>

              <div className="grid grid-cols-[1fr_120px] gap-3 mb-3">
                <div>
                  <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                    Hostname
                  </label>
                  <input
                    type="text"
                    value={h.hostname}
                    onChange={(e) => updateDraft(host.id, { hostname: e.target.value })}
                    onBlur={() => commitDraft(host.id)}
                    placeholder="e.g. dev.example.com"
                    className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                               text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                  />
                </div>
                <div>
                  <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                    Port
                  </label>
                  <input
                    type="number"
                    value={h.port}
                    onChange={(e) => updateDraft(host.id, { port: parseInt(e.target.value) || 22 })}
                    onBlur={() => commitDraft(host.id)}
                    className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                               text-gray-200 font-mono focus:border-white/[0.15] focus:outline-none"
                  />
                </div>
              </div>

              <div className="mb-3">
                <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                  Username
                </label>
                <input
                  type="text"
                  value={h.user}
                  onChange={(e) => updateDraft(host.id, { user: e.target.value })}
                  onBlur={() => commitDraft(host.id)}
                  placeholder="e.g. ubuntu"
                  className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                             text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                />
              </div>

              {/* Auth method selector */}
              <div className="mb-3">
                <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1.5 block">
                  Authentication
                </label>
                <div className="flex bg-white/[0.04] rounded-md p-0.5 gap-0.5">
                  {AUTH_METHODS.map((m) => (
                    <button
                      key={m.value}
                      onClick={() => handleAuthMethodChange(host.id, m.value)}
                      className={`flex-1 px-2 py-1.5 rounded text-xs font-medium transition-colors ${
                        authMethod === m.value
                          ? 'bg-white/[0.1] text-white'
                          : 'text-gray-500 hover:text-gray-300'
                      }`}
                    >
                      {m.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* Conditional credential fields */}
              {authMethod === 'agent' && (
                <p className="text-[11px] text-gray-600 mb-3">
                  Uses your running SSH agent (ssh-agent)
                </p>
              )}

              {authMethod === 'password' && (
                <div className="mb-3">
                  <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                    Password
                  </label>
                  <input
                    type="password"
                    value={passwordInputs[host.id] ?? ''}
                    onChange={(e) =>
                      setPasswordInputs((prev) => ({ ...prev, [host.id]: e.target.value }))
                    }
                    onBlur={() => handlePasswordCommit(host.id)}
                    placeholder={h.encryptedPassword ? '(saved)' : 'Enter password'}
                    className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                               text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                  />
                  <p className="text-[10px] text-gray-600 mt-1">Encrypted with your OS keychain</p>
                </div>
              )}

              {authMethod === 'key-stored' && (
                <div className="mb-3">
                  <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                    Stored Key
                  </label>
                  <select
                    value={h.credentialId || ''}
                    onChange={(e) => handleStoredKeyChange(host.id, e.target.value)}
                    className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                               text-gray-200 focus:border-white/[0.15] focus:outline-none"
                  >
                    <option value="">Select a key...</option>
                    {storedKeys.map((k) => (
                      <option key={k.id} value={k.id}>
                        {k.label}
                        {k.keyType ? ` (${k.keyType})` : ''}
                      </option>
                    ))}
                  </select>
                  {storedKeys.length === 0 && (
                    <p className="text-[10px] text-gray-600 mt-1">
                      No stored keys. Add keys in Settings &gt; SSH Keys.
                    </p>
                  )}
                </div>
              )}

              {authMethod === 'key-file' && (
                <div className="mb-3">
                  <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                    SSH Key Path
                  </label>
                  <div className="flex gap-1.5">
                    <input
                      type="text"
                      value={h.sshKeyPath || ''}
                      onChange={(e) => updateDraft(host.id, { sshKeyPath: e.target.value })}
                      onBlur={() => commitDraft(host.id)}
                      placeholder="~/.ssh/id_ed25519"
                      className="flex-1 min-w-0 px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                                 text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                    />
                    <button
                      onClick={() => handleBrowseKey(host.id)}
                      className="px-2 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md
                                 text-gray-400 hover:text-white hover:bg-white/[0.08] transition-colors shrink-0"
                    >
                      <Key size={14} strokeWidth={1.5} />
                    </button>
                  </div>
                </div>
              )}

              <div>
                <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                  Extra SSH Options
                </label>
                <input
                  type="text"
                  value={h.sshOptions || ''}
                  onChange={(e) => updateDraft(host.id, { sshOptions: e.target.value })}
                  onBlur={() => commitDraft(host.id)}
                  placeholder="e.g. -o ForwardAgent=yes"
                  className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                             text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                />
              </div>

              <div className="mt-3 flex items-center gap-3">
                <div className="text-[11px] text-gray-600 font-mono flex-1">{getSshPreview(h)}</div>
                <button
                  onClick={() => handleTestConnection(host.id)}
                  disabled={testResults[host.id]?.status === 'testing'}
                  className="flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] font-medium rounded-md
                             bg-white/[0.04] border border-white/[0.08] text-gray-400
                             hover:bg-white/[0.08] hover:text-white transition-colors
                             disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                >
                  {testResults[host.id]?.status === 'testing' ? (
                    <Loader2 size={12} className="animate-spin" />
                  ) : (
                    <Wifi size={12} />
                  )}
                  Test
                </button>
              </div>
              {testResults[host.id] && testResults[host.id].status !== 'testing' && (
                <div
                  className={`mt-2 flex items-center gap-1.5 text-[11px] px-2.5 py-1.5 rounded-md ${
                    testResults[host.id].status === 'success'
                      ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                      : 'bg-red-500/10 text-red-400 border border-red-500/20'
                  }`}
                >
                  {testResults[host.id].status === 'success' ? (
                    <CheckCircle2 size={12} className="shrink-0" />
                  ) : (
                    <XCircle size={12} className="shrink-0" />
                  )}
                  <span className="truncate">{testResults[host.id].message}</span>
                </div>
              )}
            </div>
          )
        })}

        {hosts.length === 0 && (
          <div className="text-center py-8 text-sm text-gray-600">No remote hosts configured</div>
        )}

        <button
          onClick={handleAddHost}
          className="w-full py-2.5 border border-dashed border-white/[0.08] rounded-lg
                     text-sm text-gray-500 hover:text-white hover:border-white/[0.15]
                     hover:bg-white/[0.02] transition-colors flex items-center justify-center gap-2"
        >
          <Plus size={14} strokeWidth={2} />
          Add Host
        </button>
      </div>
    </div>
  )
}
