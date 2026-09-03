import { useState, useEffect } from 'react'
import { useAppStore } from '../../stores'
import { Check, AlertCircle } from 'lucide-react'
import type { ConnectorManifest, TaskStatus } from '../../../shared/types'
import { ConnectorIcon } from '../ConnectorIcon'
import { glyphForConnectorId } from '../../lib/use-connections'
import { SdkConnectorForm } from './SdkConnectorForm'
import { DynamicField } from './DynamicField'

/** Connector id the generic MCP stdio connection registers under. */
export const MCP_CONNECTOR_ID = 'mcp'

/** A connector as the settings list knows it, manifest included. */
export interface ConnectorInfo {
  id: string
  name: string
  icon: string
  capabilities: string[]
  manifest: ConnectorManifest
}

/**
 * The form that turns a connector into a connection of it.
 *
 * Lifted out of the settings page so the workflow editor can offer the same
 * form against a template's unmet requirement: connecting is the same act
 * wherever it is asked for, and a second copy would drift from this one.
 */
export function AddConnectionForm({
  connector,
  initialAuth,
  extraFilters,
  startManual,
  onDone,
  onCancel
}: {
  connector: ConnectorInfo
  /** Pre-filled fields, so a listed server arrives with its launch line written. */
  initialAuth?: Record<string, string>
  /** Stamped onto the saved connection, whatever the form asked for. */
  extraFilters?: Record<string, string>
  startManual?: boolean
  onDone: () => void
  onCancel: () => void
}) {
  const projects = useAppStore((s) => s.config?.projects || [])
  const manifest = connector.manifest
  // Asked for by the connector rather than decided here. This was a test on the
  // id of the one built-in that wanted it; a connector that ships as a pack
  // could not have it at all, which made the built-in irreplaceable.
  const usesRepoDetect = manifest.detectRepo === true
  // An MCP connection is either a connector package that describes itself or
  // a raw server the user wires up by hand. The first covers most cases, so
  // it leads.
  const isMcp = connector.id === MCP_CONNECTOR_ID
  const [fromPackage, setFromPackage] = useState(isMcp && startManual !== true)

  const [selectedProject, setSelectedProject] = useState(projects[0]?.name || '')
  const [detectedRepo, setDetectedRepo] = useState<{
    owner: string
    repo: string
  } | null>(null)
  const [detecting, setDetecting] = useState(false)
  const [auth, setAuth] = useState<Record<string, string>>(initialAuth ?? {})
  const [filters, setFilters] = useState<Record<string, string>>({})
  const [manualRepo, setManualRepo] = useState<{ owner: string; repo: string }>({
    owner: '',
    repo: ''
  })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!usesRepoDetect) return
    const project = projects.find((p) => p.name === selectedProject)
    if (!project) return
    /* eslint-disable react-hooks/set-state-in-effect -- intentional: marks detection in flight before querying the main process */
    setDetecting(true)
    setDetectedRepo(null)
    /* eslint-enable react-hooks/set-state-in-effect */
    window.api.detectRepo(project.path).then((result) => {
      setDetectedRepo(result)
      setDetecting(false)
    })
  }, [selectedProject, projects, usesRepoDetect])

  const statusMapping: Record<string, TaskStatus> = {}
  for (const opt of manifest.statusMapping || []) {
    statusMapping[opt.upstream] = opt.suggestedLocal
  }

  const missingAuth = (manifest.auth ?? []).some((f) => f.required && !auth[f.key]?.trim())
  const manualRepoValid = manualRepo.owner.trim().length > 0 && manualRepo.repo.trim().length > 0
  const canSave = usesRepoDetect ? !!detectedRepo || manualRepoValid : !missingAuth

  const handleSave = async () => {
    setError(null)
    setSaving(true)
    try {
      // Encrypt any password-typed auth fields via Electron's safeStorage
      // BEFORE they touch the DB. Plaintext never leaves this call.
      const encryptedAuth: Record<string, string> = {}
      for (const field of manifest.auth ?? []) {
        const v = auth[field.key]
        if (!v) continue
        if (field.type === 'password') {
          try {
            encryptedAuth[field.key] = await window.api.encryptString(v)
          } catch (err) {
            throw new Error(
              `Could not encrypt ${field.label}: ${err instanceof Error ? err.message : String(err)}. ` +
                `OS keychain access may be unavailable.`,
              { cause: err }
            )
          }
        } else {
          encryptedAuth[field.key] = v
        }
      }

      const connectionFilters: Record<string, unknown> = {
        ...encryptedAuth,
        ...filters,
        ...extraFilters
      }
      let name: string
      if (usesRepoDetect) {
        const owner = detectedRepo?.owner ?? manualRepo.owner.trim()
        const repo = detectedRepo?.repo ?? manualRepo.repo.trim()
        connectionFilters.owner = owner
        connectionFilters.repo = repo
        name = `${owner}/${repo}`
      } else {
        name =
          auth.profileName?.trim() ||
          (filters.teamKey && `${connector.name}: ${filters.teamKey}`) ||
          `${connector.name}${selectedProject ? ` · ${selectedProject}` : ''}`
      }
      await window.api.createConnection({
        connectorId: connector.id,
        name,
        filters: connectionFilters,
        syncIntervalMinutes: 5,
        statusMapping,
        executionProject: selectedProject
      })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="mb-6 px-4 py-3 bg-white/[0.03] border border-white/[0.08] rounded-sm">
      <div className="flex items-center gap-2 mb-3">
        <ConnectorIcon
          connectorId={connector.id}
          icon={glyphForConnectorId(connector.id)}
          size={14}
          className="text-gray-400"
        />
        <h3 className="text-sm font-medium text-gray-200">Connect {connector.name}</h3>
      </div>

      {isMcp && (
        <div className="flex gap-1 mb-3">
          {[
            { key: true, label: 'From a package' },
            { key: false, label: 'Manual setup' }
          ].map((tab) => (
            <button
              key={String(tab.key)}
              onClick={() => setFromPackage(tab.key)}
              className={`px-3 py-1 text-xs rounded-sm transition-colors ${
                fromPackage === tab.key
                  ? 'bg-white/[0.1] text-gray-200'
                  : 'text-gray-500 hover:text-gray-300'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      )}

      {isMcp && fromPackage ? (
        <SdkConnectorForm onDone={onDone} onCancel={onCancel} />
      ) : (
        <div className="space-y-3">
          <div>
            <label className="block text-xs text-gray-500 mb-1">Project</label>
            <select
              value={selectedProject}
              onChange={(e) => setSelectedProject(e.target.value)}
              className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
            >
              {projects.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>

          {usesRepoDetect && (
            <>
              <div className="text-xs">
                {detecting && <span className="text-gray-500">Detecting repository...</span>}
                {detectedRepo && (
                  <span className="text-green-400 flex items-center gap-1">
                    <Check size={12} /> Detected: {detectedRepo.owner}/{detectedRepo.repo}
                  </span>
                )}
                {!detecting && !detectedRepo && selectedProject && (
                  <span className="text-amber-400">
                    No GitHub repo detected. Enter the repo manually below.
                  </span>
                )}
              </div>

              {/* Manual fallback — only visible when auto-detect failed. Covers
                GH Enterprise, non-standard remotes, and detached repos. */}
              {!detecting && !detectedRepo && (
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Owner</label>
                    <input
                      type="text"
                      value={manualRepo.owner}
                      onChange={(e) =>
                        setManualRepo((prev) => ({ ...prev, owner: e.target.value }))
                      }
                      placeholder="e.g. octocat"
                      className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Repository</label>
                    <input
                      type="text"
                      value={manualRepo.repo}
                      onChange={(e) => setManualRepo((prev) => ({ ...prev, repo: e.target.value }))}
                      placeholder="e.g. hello-world"
                      className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
                    />
                  </div>
                </div>
              )}
            </>
          )}

          {(manifest.auth ?? []).map((field) => (
            <DynamicField
              key={field.key}
              field={field}
              value={auth[field.key] || ''}
              onChange={(v) => setAuth((prev) => ({ ...prev, [field.key]: v }))}
            />
          ))}

          {(manifest.taskFilters || []).map((field) => (
            <DynamicField
              key={field.key}
              field={field}
              value={filters[field.key] || ''}
              onChange={(v) => setFilters((prev) => ({ ...prev, [field.key]: v }))}
            />
          ))}

          {error && (
            <div className="text-[11px] text-red-400 flex items-start gap-1">
              <AlertCircle size={12} className="mt-0.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          <div className="flex gap-2 pt-1">
            <button
              onClick={handleSave}
              disabled={saving || !canSave}
              className="px-4 py-1.5 text-sm bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50"
            >
              {saving ? 'Connecting...' : 'Connect'}
            </button>
            <button
              onClick={onCancel}
              className="px-4 py-1.5 text-sm text-gray-500 hover:text-gray-300 transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
