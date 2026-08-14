import { useState, useEffect, useCallback, DragEvent } from 'react'
import { Trash2, Plus, Upload, FileKey } from 'lucide-react'
import type { SSHKeyMeta } from '../../../shared/types'

export function KeyVaultSettings() {
  const [keys, setKeys] = useState<SSHKeyMeta[]>([])
  const [safeStorageAvailable, setSafeStorageAvailable] = useState(true)
  const [showAddForm, setShowAddForm] = useState(false)
  const [isDragOver, setIsDragOver] = useState(false)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  // New key form state
  const [label, setLabel] = useState('')
  const [privateKey, setPrivateKey] = useState('')
  const [publicKey, setPublicKey] = useState('')
  const [certificate, setCertificate] = useState('')

  const loadKeys = useCallback(async () => {
    try {
      const result = await window.api.listSSHKeys()
      setKeys(result)
    } catch {
      /* ignore */
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    window.api
      .listSSHKeys()
      .then((result) => {
        if (!cancelled) setKeys(result)
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

  const handleSaveKey = async (): Promise<void> => {
    if (!privateKey.trim()) return
    await window.api.storeSSHKey({
      label: label.trim() || 'Untitled Key',
      privateKey: privateKey.trim(),
      publicKey: publicKey.trim() || undefined,
      certificate: certificate.trim() || undefined
    })
    setLabel('')
    setPrivateKey('')
    setPublicKey('')
    setCertificate('')
    setShowAddForm(false)
    loadKeys()
  }

  const handleImportFile = async (): Promise<void> => {
    const filePath = await window.api.openFileDialog()
    if (filePath) {
      await window.api.importSSHKeyFile({ filePath })
      loadKeys()
    }
  }

  const handleDelete = async (id: string): Promise<void> => {
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
      if (filePath) {
        await window.api.importSSHKeyFile({ filePath })
      }
    }
    loadKeys()
  }

  const truncateKey = (key: string): string => {
    if (key.length <= 60) return key
    return key.slice(0, 30) + '...' + key.slice(-25)
  }

  const disabled = !safeStorageAvailable

  return (
    <div>
      <h2 className="text-xl font-semibold text-white mb-1">SSH Key Vault</h2>
      <p className="text-sm text-gray-500 mb-6">
        Store SSH private keys encrypted with your OS keychain
      </p>

      {!safeStorageAvailable && (
        <div
          className="mb-4 px-4 py-3 rounded-lg border border-amber-500/30 text-sm text-amber-400"
          style={{ background: 'rgba(245, 158, 11, 0.08)' }}
        >
          Keychain encryption is not available on this system. Install libsecret (gnome-keyring) to
          use the SSH key vault.
        </div>
      )}

      <div
        className="space-y-3"
        onDragOver={disabled ? undefined : handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={disabled ? undefined : handleDrop}
      >
        {keys.map((key) => (
          <div
            key={key.id}
            className="border border-white/[0.06] rounded-lg p-4"
            style={{ background: 'var(--color-surface-sunken)' }}
          >
            <div className="flex items-center gap-3">
              <FileKey size={18} strokeWidth={1.5} className="text-blue-400 shrink-0" />
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium text-gray-200">{key.label}</div>
                <div className="flex items-center gap-2 mt-0.5">
                  {key.keyType && (
                    <span className="text-[10px] uppercase tracking-wider font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-gray-400">
                      {key.keyType}
                    </span>
                  )}
                  {key.publicKey && (
                    <span className="text-[11px] text-gray-600 font-mono truncate">
                      {truncateKey(key.publicKey)}
                    </span>
                  )}
                </div>
              </div>
              {confirmDeleteId === key.id ? (
                <div className="flex items-center gap-1.5">
                  <button
                    onClick={() => handleDelete(key.id)}
                    className="px-2 py-1 text-xs text-red-400 bg-red-400/10 border border-red-400/20 rounded-md hover:bg-red-400/20 transition-colors"
                  >
                    Delete
                  </button>
                  <button
                    onClick={() => setConfirmDeleteId(null)}
                    className="px-2 py-1 text-xs text-gray-400 hover:text-white transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => setConfirmDeleteId(key.id)}
                  className="text-gray-600 hover:text-red-400 p-1 transition-colors"
                >
                  <Trash2 size={14} strokeWidth={1.5} />
                </button>
              )}
            </div>
          </div>
        ))}

        {keys.length === 0 && !showAddForm && (
          <div className="text-center py-8 text-sm text-gray-600">No stored SSH keys</div>
        )}

        {showAddForm && (
          <div
            className="border border-white/[0.08] rounded-lg p-4 space-y-3"
            style={{ background: 'var(--color-surface-sunken)' }}
          >
            <div className="text-sm font-medium text-gray-200 mb-3">New Key</div>
            <div>
              <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                Label
              </label>
              <input
                type="text"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="e.g. Work Server Key"
                className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                           text-gray-200 placeholder-gray-600 focus:border-white/[0.15] focus:outline-none"
              />
            </div>
            <div>
              <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                Private key *
              </label>
              <textarea
                value={privateKey}
                onChange={(e) => setPrivateKey(e.target.value)}
                placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                rows={4}
                className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <div>
              <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                Public key
              </label>
              <textarea
                value={publicKey}
                onChange={(e) => setPublicKey(e.target.value)}
                placeholder="ssh-ed25519 AAAA..."
                rows={2}
                className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <div>
              <label className="text-[11px] text-gray-500 uppercase tracking-wider mb-1 block">
                Certificate
              </label>
              <textarea
                value={certificate}
                onChange={(e) => setCertificate(e.target.value)}
                placeholder="Optional"
                rows={2}
                className="w-full px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                           text-gray-200 font-mono placeholder-gray-600 focus:border-white/[0.15] focus:outline-none resize-none"
              />
            </div>
            <p className="text-[11px] text-gray-600">
              If your key has a passphrase, remove it first with{' '}
              <code className="text-gray-500">ssh-keygen -p</code>
            </p>
            <div className="flex gap-2">
              <button
                onClick={handleSaveKey}
                disabled={!privateKey.trim()}
                className="px-4 py-1.5 text-sm font-medium bg-blue-600 hover:bg-blue-500 disabled:opacity-40
                           disabled:cursor-not-allowed text-white rounded-md transition-colors"
              >
                Save Key
              </button>
              <button
                onClick={() => {
                  setShowAddForm(false)
                  setLabel('')
                  setPrivateKey('')
                  setPublicKey('')
                  setCertificate('')
                }}
                className="px-4 py-1.5 text-sm text-gray-400 hover:text-white transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Drag and drop zone + import */}
        <div
          className={`border border-dashed rounded-lg p-6 text-center transition-colors ${
            isDragOver
              ? 'border-blue-400/50 bg-blue-400/5'
              : 'border-white/[0.08] hover:border-white/[0.15]'
          } ${disabled ? 'opacity-40 cursor-not-allowed' : ''}`}
        >
          <Upload
            size={24}
            strokeWidth={1.5}
            className={`mx-auto mb-2 ${isDragOver ? 'text-blue-400' : 'text-gray-600'}`}
          />
          <p className="text-sm text-gray-500 mb-3">Drag and drop a private key file to import</p>
          <button
            onClick={handleImportFile}
            disabled={disabled}
            className="px-4 py-2 text-sm font-medium bg-emerald-600 hover:bg-emerald-500
                       disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors"
          >
            Import from key file
          </button>
        </div>

        {!showAddForm && (
          <button
            onClick={() => setShowAddForm(true)}
            disabled={disabled}
            className="w-full py-2.5 border border-dashed border-white/[0.08] rounded-lg
                       text-sm text-gray-500 hover:text-white hover:border-white/[0.15]
                       hover:bg-white/[0.02] transition-colors flex items-center justify-center gap-2
                       disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Plus size={14} strokeWidth={2} />
            Paste Key Manually
          </button>
        )}
      </div>
    </div>
  )
}
