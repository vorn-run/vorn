import { useState, useEffect, useCallback, DragEvent } from 'react'
import { useAppStore } from '../../stores'
import type { RemoteHost, AuthMethod, SSHKeyMeta } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import {
  Server,
  Trash2,
  Plus,
  Key,
  FileKey,
  Upload,
  ChevronDown,
  ChevronRight,
  Wifi,
  Loader2,
  CheckCircle2,
  XCircle
} from 'lucide-react'

const AUTH_METHODS: { value: AuthMethod; label: string }[] = [
  { value: 'agent', label: 'SSH Agent' },
  { value: 'password', label: 'Password' },
  { value: 'key-stored', label: 'Stored Key' },
  { value: 'key-file', label: 'Key File' }
]

export function SSHSettings() {
  const config = useAppStore((s) => s.config)
  const addRemoteHost = useAppStore((s) => s.addRemoteHost)
  const removeRemoteHost = useAppStore((s) => s.removeRemoteHost)
  const updateRemoteHost = useAppStore((s) => s.updateRemoteHost)

  const [expandedHosts, setExpandedHosts] = useState<Set<string>>(new Set())
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(new Set())
  const [storedKeys, setStoredKeys] = useState<SSHKeyMeta[]>([])
  const [passwordInputs, setPasswordInputs] = useState<Record<string, string>>({})
  const [safeStorageAvailable, setSafeStorageAvailable] = useState(true)
  const [isDragOver, setIsDragOver] = useState(false)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)
  const [testResults, setTestResults] = useState<
    Record<string, { status: 'testing' | 'success' | 'error'; message?: string }>
  >({})

  // New key form
  const [showAddKeyForm, setShowAddKeyForm] = useState(false)
  const [keyLabel, setKeyLabel] = useState('')
  const [keyPrivate, setKeyPrivate] = useState('')
  const [keyPublic, setKeyPublic] = useState('')
  const [keyCertificate, setKeyCertificate] = useState('')

  const loadKeys = useCallback(async () => {
    try {
      const result = await window.api.listSSHKeys()
      setStoredKeys(result)
    } catch {
      /* ignore */
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    window.api
      .listSSHKeys()
      .then((result) => {
        if (!cancelled) setStoredKeys(result)
      })
      .catch(() => {})
    window.api
      .isSafeStorageAvailable()
      .then((v) => {
        if (!cancelled) setSafeStorageAvailable(v)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  if (!config) return null

  const hosts = config.remoteHosts || []

  const toggleHost = (id: string): void => {
    setExpandedHosts((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const toggleKey = (id: string): void => {
    setExpandedKeys((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  // ─── Host helpers ───
  const updateHost = (id: string, patch: Partial<RemoteHost>): void => {
    const host = hosts.find((h) => h.id === id)
    if (!host) return
    updateRemoteHost(id, { ...host, ...patch })
  }

  const handleAddHost = (): void => {
    const id = crypto.randomUUID()
    addRemoteHost({ id, label: 'New Host', hostname: '', user: '', port: 22, authMethod: 'agent' })
    setExpandedHosts((prev) => new Set(prev).add(id))
  }

  const handleDeleteHost = (id: string): void => {
    removeRemoteHost(id)
    setConfirmDeleteId(null)
  }

  const handleBrowseKey = async (hostId: string): Promise<void> => {
    const filePath = await window.api.openFileDialog()
    if (filePath) updateHost(hostId, { sshKeyPath: filePath })
  }

  const handleAuthMethodChange = (hostId: string, method: AuthMethod): void => {
    const patch: Partial<RemoteHost> = { authMethod: method }
    if (method !== 'key-file') patch.sshKeyPath = undefined
    if (method !== 'key-stored') patch.credentialId = undefined
    if (method !== 'password') patch.encryptedPassword = undefined
    updateHost(hostId, patch)
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
      updateHost(hostId, { encryptedPassword: encrypted })
      setPasswordInputs((prev) => {
        const next = { ...prev }
        delete next[hostId]
        return next
      })
    } catch {
      /* encryption not available */
    }
  }

  const handleStoredKeyChange = (hostId: string, credentialId: string): void => {
    updateHost(hostId, { credentialId })
  }

  // ─── Key helpers ───
  const handleSaveKey = async (): Promise<void> => {
    if (!keyPrivate.trim()) return
    await window.api.storeSSHKey({
      label: keyLabel.trim() || 'Untitled Key',
      privateKey: keyPrivate.trim(),
      publicKey: keyPublic.trim() || undefined,
      certificate: keyCertificate.trim() || undefined
    })
    setKeyLabel('')
    setKeyPrivate('')
    setKeyPublic('')
    setKeyCertificate('')
    setShowAddKeyForm(false)
    loadKeys()
  }

  const handleImportKeyFile = async (): Promise<void> => {
    const filePath = await window.api.openFileDialog()
    if (filePath) {
      await window.api.importSSHKeyFile({ filePath })
      loadKeys()
    }
  }

  const handleDeleteKey = async (id: string): Promise<void> => {
    await window.api.deleteSSHKey(id)
    setConfirmDeleteId(null)
    loadKeys()
  }

  const handleDragOver = (e: DragEvent): void => {
    e.preventDefault()
    e.stopPropagation()
    setIsDragOver(true)
  }
  const handleDragLeave = (e: DragEvent): void => {
    e.preventDefault()
    e.stopPropagation()
    setIsDragOver(false)
  }
  const handleDrop = async (e: DragEvent): Promise<void> => {
    e.preventDefault()
    e.stopPropagation()
    setIsDragOver(false)
    const files = e.dataTransfer?.files
    if (!files || files.length === 0) return
    for (const file of Array.from(files)) {
      const filePath = (file as File & { path?: string }).path
      if (filePath) await window.api.importSSHKeyFile({ filePath })
    }
    loadKeys()
  }

  const truncateKey = (key: string): string => {
    if (key.length <= 60) return key
    return key.slice(0, 30) + '...' + key.slice(-25)
  }

  const handleTestConnection = async (hostId: string): Promise<void> => {
    const host = hosts.find((h) => h.id === hostId)
    if (!host) return
    const effective = host
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

  const hostsUsingKey = (keyId: string): RemoteHost[] =>
    hosts.filter((h) => h.authMethod === 'key-stored' && h.credentialId === keyId)

  return (
    <div>
      <SettingsPageHeader
        title="SSH & Hosts"
        description="Configure remote hosts and SSH keys for running agents on remote machines"
      />

      {!safeStorageAvailable && (
        <div
          className="mb-6 px-4 py-3 rounded-lg border border-amber-500/30 text-sm text-amber-400"
          style={{ background: 'rgba(245, 158, 11, 0.08)' }}
        >
          Keychain encryption is not available on this system. Install libsecret (gnome-keyring) to
          use the SSH key vault.
        </div>
      )}

      {/* ─── Hosts section ─── */}
      <div className="mb-8">
        <div className="flex items-center gap-2 mb-3">
          <Server size={12} className="text-gray-600" />
          <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider">
            Remote Hosts
          </h3>
          <span className="text-[10px] text-gray-600">{hosts.length}</span>
        </div>

        <div className="space-y-2">
          {hosts.map((host) => {
            const h = host
            const isExpanded = expandedHosts.has(host.id)
            const authMethod = h.authMethod ?? 'agent'
            const subtitle =
              h.user && h.hostname
                ? `${h.user}@${h.hostname}${h.port !== 22 ? ':' + h.port : ''}`
                : h.hostname || 'Not configured'

            return (
              <div
                key={host.id}
                className="rounded-xl border border-white/[0.06] hover:border-white/[0.1] transition-all overflow-hidden"
                style={{ background: 'var(--color-surface-sunken)' }}
              >
                {/* Collapsed row */}
                <button
                  onClick={() => toggleHost(host.id)}
                  className="flex items-center gap-3 w-full px-4 py-3 text-left"
                >
                  <div className="w-9 h-9 rounded-lg flex items-center justify-center shrink-0 bg-blue-500/10">
                    <Server size={18} strokeWidth={1.5} className="text-blue-400" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-100">{h.label}</div>
                    <div className="text-[11px] text-gray-500 font-mono truncate">{subtitle}</div>
                  </div>
                  {isExpanded ? (
                    <ChevronDown size={14} className="text-gray-600 shrink-0" />
                  ) : (
                    <ChevronRight size={14} className="text-gray-600 shrink-0" />
                  )}
                </button>

                {/* Expanded form */}
                {isExpanded && (
                  <div className="px-4 pb-4 pt-1 space-y-3">
                    {/* Label */}
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                        Label
                      </label>
                      <input
                        type="text"
                        value={h.label}
                        onChange={(e) => updateHost(host.id, { label: e.target.value })}
                        placeholder="Host label"
                        className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                   text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                      />
                    </div>

                    {/* Hostname + Port */}
                    <div className="grid grid-cols-[1fr_100px] gap-2">
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Hostname
                        </label>
                        <input
                          type="text"
                          value={h.hostname}
                          onChange={(e) => updateHost(host.id, { hostname: e.target.value })}
                          placeholder="e.g. dev.example.com"
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                     text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                        />
                      </div>
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Port
                        </label>
                        <input
                          type="number"
                          value={h.port}
                          onChange={(e) =>
                            updateHost(host.id, { port: parseInt(e.target.value) || 22 })
                          }
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                     text-gray-200 font-mono focus:border-white/[0.15] focus:outline-none"
                        />
                      </div>
                    </div>

                    {/* Username */}
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                        Username
                      </label>
                      <input
                        type="text"
                        value={h.user}
                        onChange={(e) => updateHost(host.id, { user: e.target.value })}
                        placeholder="e.g. ubuntu"
                        className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                   text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                      />
                    </div>

                    {/* Auth method */}
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1.5 block">
                        Authentication
                      </label>
                      <div className="flex bg-white/[0.04] rounded-md p-0.5 gap-0.5">
                        {AUTH_METHODS.map((m) => (
                          <button
                            key={m.value}
                            onClick={() => handleAuthMethodChange(host.id, m.value)}
                            className={`flex-1 px-2 py-1.5 rounded text-[11px] font-medium transition-colors ${
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
                      <p className="text-[11px] text-gray-600">
                        Uses your running SSH agent (ssh-agent)
                      </p>
                    )}

                    {authMethod === 'password' && (
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
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
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                     text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                        />
                        <p className="text-[10px] text-gray-600 mt-1">
                          Encrypted with your OS keychain
                        </p>
                      </div>
                    )}

                    {authMethod === 'key-stored' && (
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Stored Key
                        </label>
                        <select
                          value={h.credentialId || ''}
                          onChange={(e) => handleStoredKeyChange(host.id, e.target.value)}
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
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
                            No stored keys yet. Add one below.
                          </p>
                        )}
                      </div>
                    )}

                    {authMethod === 'key-file' && (
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          SSH Key Path
                        </label>
                        <div className="flex gap-1.5">
                          <input
                            type="text"
                            value={h.sshKeyPath || ''}
                            onChange={(e) => updateHost(host.id, { sshKeyPath: e.target.value })}
                            placeholder="~/.ssh/id_ed25519"
                            className="flex-1 min-w-0 px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                       text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                          />
                          <button
                            onClick={() => handleBrowseKey(host.id)}
                            className="px-2 py-1.5 bg-black/30 border border-white/[0.06] rounded-md
                                       text-gray-400 hover:text-white hover:bg-white/[0.08] transition-colors shrink-0"
                          >
                            <Key size={12} strokeWidth={1.5} />
                          </button>
                        </div>
                      </div>
                    )}

                    {/* SSH Options */}
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                        Extra SSH Options
                      </label>
                      <input
                        type="text"
                        value={h.sshOptions || ''}
                        onChange={(e) => updateHost(host.id, { sshOptions: e.target.value })}
                        placeholder="e.g. -o ForwardAgent=yes"
                        className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                   text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
                      />
                    </div>

                    {/* Test + Delete */}
                    <div className="pt-2 flex items-center justify-between">
                      <button
                        onClick={() => handleTestConnection(host.id)}
                        disabled={testResults[host.id]?.status === 'testing'}
                        className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md
                                   bg-white/[0.04] border border-white/[0.06] text-gray-400
                                   hover:bg-white/[0.08] hover:text-white transition-colors
                                   disabled:opacity-50 disabled:cursor-not-allowed"
                      >
                        {testResults[host.id]?.status === 'testing' ? (
                          <Loader2 size={11} className="animate-spin" />
                        ) : (
                          <Wifi size={11} />
                        )}
                        Test Connection
                      </button>
                      {confirmDeleteId === host.id ? (
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleDeleteHost(host.id)}
                            className="px-3 py-1.5 text-xs text-red-400 bg-red-400/10 border border-red-400/20
                                       rounded-md hover:bg-red-400/20 transition-colors"
                          >
                            Confirm Delete
                          </button>
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="px-3 py-1.5 text-xs text-gray-400 hover:text-white transition-colors"
                          >
                            Cancel
                          </button>
                        </div>
                      ) : (
                        <button
                          onClick={() => setConfirmDeleteId(host.id)}
                          className="flex items-center gap-1.5 px-3 py-1.5 text-xs text-gray-500 hover:text-red-400
                                     bg-white/[0.03] hover:bg-red-400/10 border border-white/[0.04] hover:border-red-400/20
                                     rounded-md transition-all"
                        >
                          <Trash2 size={11} strokeWidth={1.5} />
                          Delete Host
                        </button>
                      )}
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
                        <span>{testResults[host.id].message}</span>
                      </div>
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </div>

        {/* Add host */}
        <button
          onClick={handleAddHost}
          className="w-full mt-2 py-2.5 border border-dashed border-white/[0.08] rounded-lg
                     text-sm text-gray-500 hover:text-white hover:border-white/[0.15]
                     hover:bg-white/[0.02] transition-colors flex items-center justify-center gap-2"
        >
          <Plus size={14} strokeWidth={2} />
          Add Host
        </button>
      </div>

      {/* ─── Keys section ─── */}
      <div
        onDragOver={safeStorageAvailable ? handleDragOver : undefined}
        onDragLeave={handleDragLeave}
        onDrop={safeStorageAvailable ? handleDrop : undefined}
      >
        <div className="flex items-center gap-2 mb-3">
          <FileKey size={12} className="text-gray-600" />
          <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider">SSH Keys</h3>
          <span className="text-[10px] text-gray-600">{storedKeys.length}</span>
        </div>

        {isDragOver && (
          <div className="mb-2 py-6 border-2 border-dashed border-blue-400/40 rounded-lg bg-blue-400/5 text-center">
            <p className="text-sm text-blue-400">Drop key file to import</p>
          </div>
        )}

        <div className="space-y-2">
          {storedKeys.map((sshKey) => {
            const isExpanded = expandedKeys.has(sshKey.id)
            const usedBy = hostsUsingKey(sshKey.id)

            return (
              <div
                key={sshKey.id}
                className="rounded-xl border border-white/[0.06] hover:border-white/[0.1] transition-all overflow-hidden"
                style={{ background: 'var(--color-surface-sunken)' }}
              >
                {/* Collapsed row */}
                <button
                  onClick={() => toggleKey(sshKey.id)}
                  className="flex items-center gap-3 w-full px-4 py-3 text-left"
                >
                  <div className="w-9 h-9 rounded-lg flex items-center justify-center shrink-0 bg-amber-500/10">
                    <FileKey size={18} strokeWidth={1.5} className="text-amber-400" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-gray-100">{sshKey.label}</span>
                      {sshKey.keyType && (
                        <span className="text-[10px] uppercase tracking-wider font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-gray-400">
                          {sshKey.keyType}
                        </span>
                      )}
                    </div>
                    {sshKey.publicKey && (
                      <div className="text-[11px] text-gray-600 font-mono truncate mt-0.5">
                        {truncateKey(sshKey.publicKey)}
                      </div>
                    )}
                  </div>
                  {isExpanded ? (
                    <ChevronDown size={14} className="text-gray-600 shrink-0" />
                  ) : (
                    <ChevronRight size={14} className="text-gray-600 shrink-0" />
                  )}
                </button>

                {/* Expanded detail */}
                {isExpanded && (
                  <div className="px-4 pb-4 pt-1 space-y-3">
                    {sshKey.publicKey && (
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Public Key
                        </label>
                        <div className="px-2.5 py-2 bg-black/30 border border-white/[0.06] rounded-md text-[11px] text-gray-500 font-mono break-all leading-relaxed">
                          {sshKey.publicKey}
                        </div>
                      </div>
                    )}

                    {/* Used by hosts */}
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1.5 block">
                        Used By
                      </label>
                      {usedBy.length > 0 ? (
                        <div className="flex flex-wrap gap-1.5">
                          {usedBy.map((host) => (
                            <span
                              key={host.id}
                              className="flex items-center gap-1.5 text-[11px] text-gray-300 px-2 py-1 rounded-md bg-white/[0.04] border border-white/[0.06]"
                            >
                              <Server size={10} strokeWidth={1.5} className="text-blue-400" />
                              {host.label}
                            </span>
                          ))}
                        </div>
                      ) : (
                        <p className="text-[11px] text-gray-600">No hosts using this key</p>
                      )}
                    </div>

                    {/* Delete */}
                    <div className="pt-2 flex justify-end">
                      {confirmDeleteId === sshKey.id ? (
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleDeleteKey(sshKey.id)}
                            className="px-3 py-1.5 text-xs text-red-400 bg-red-400/10 border border-red-400/20
                                       rounded-md hover:bg-red-400/20 transition-colors"
                          >
                            Confirm Delete
                          </button>
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="px-3 py-1.5 text-xs text-gray-400 hover:text-white transition-colors"
                          >
                            Cancel
                          </button>
                        </div>
                      ) : (
                        <button
                          onClick={() => setConfirmDeleteId(sshKey.id)}
                          className="flex items-center gap-1.5 px-3 py-1.5 text-xs text-gray-500 hover:text-red-400
                                     bg-white/[0.03] hover:bg-red-400/10 border border-white/[0.04] hover:border-red-400/20
                                     rounded-md transition-all"
                        >
                          <Trash2 size={11} strokeWidth={1.5} />
                          Delete Key
                        </button>
                      )}
                    </div>
                  </div>
                )}
              </div>
            )
          })}
        </div>

        {/* Add key form */}
        {showAddKeyForm && (
          <div
            className="mt-2 rounded-xl border border-white/[0.08] p-4 space-y-3"
            style={{ background: 'var(--color-surface-sunken)' }}
          >
            <div className="text-sm font-medium text-gray-200">New Key</div>
            <div>
              <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                Label
              </label>
              <input
                type="text"
                value={keyLabel}
                onChange={(e) => setKeyLabel(e.target.value)}
                placeholder="e.g. Work Server Key"
                className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                           text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
              />
            </div>
            <div>
              <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                Private key *
              </label>
              <textarea
                value={keyPrivate}
                onChange={(e) => setKeyPrivate(e.target.value)}
                placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                rows={3}
                className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <div>
              <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                Public key
              </label>
              <textarea
                value={keyPublic}
                onChange={(e) => setKeyPublic(e.target.value)}
                placeholder="ssh-ed25519 AAAA..."
                rows={2}
                className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <div>
              <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                Certificate
              </label>
              <textarea
                value={keyCertificate}
                onChange={(e) => setKeyCertificate(e.target.value)}
                placeholder="Optional"
                rows={2}
                className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <p className="text-[10px] text-gray-600">
              If your key has a passphrase, remove it first with{' '}
              <code className="text-gray-500">ssh-keygen -p</code>
            </p>
            <div className="flex gap-2">
              <button
                onClick={handleSaveKey}
                disabled={!keyPrivate.trim()}
                className="px-4 py-1.5 text-xs font-medium bg-blue-600 hover:bg-blue-500 disabled:opacity-40
                           disabled:cursor-not-allowed text-white rounded-md transition-colors"
              >
                Save Key
              </button>
              <button
                onClick={() => {
                  setShowAddKeyForm(false)
                  setKeyLabel('')
                  setKeyPrivate('')
                  setKeyPublic('')
                  setKeyCertificate('')
                }}
                className="px-4 py-1.5 text-xs text-gray-400 hover:text-white transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Import / Paste buttons */}
        <div className="flex gap-2 mt-2">
          <button
            onClick={handleImportKeyFile}
            disabled={!safeStorageAvailable}
            className="flex-1 py-2.5 border border-dashed border-white/[0.08] rounded-lg
                       text-sm text-gray-500 hover:text-white hover:border-white/[0.15]
                       hover:bg-white/[0.02] transition-colors flex items-center justify-center gap-2
                       disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Upload size={14} strokeWidth={1.5} />
            Import Key
          </button>
          {!showAddKeyForm && (
            <button
              onClick={() => setShowAddKeyForm(true)}
              disabled={!safeStorageAvailable}
              className="flex-1 py-2.5 border border-dashed border-white/[0.08] rounded-lg
                         text-sm text-gray-500 hover:text-white hover:border-white/[0.15]
                         hover:bg-white/[0.02] transition-colors flex items-center justify-center gap-2
                         disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <Plus size={14} strokeWidth={2} />
              Paste Key
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
