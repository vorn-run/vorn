import { useCallback, useEffect, useRef, useState } from 'react'
import { AlertCircle, Check, Loader2, Search } from 'lucide-react'
import {
  borrowableFromManifest,
  type AuthProbeReport,
  type ConnectorCatalogItem,
  type InstalledConnectorPack,
  type SdkConnectorManifest,
  type SdkTrigger,
  type TaskStatus
} from '../../../shared/types'
import { useAppStore } from '../../stores'
import { parseLaunchSpec } from './parse-launch-spec'
import { ConnectorIcon } from '../ConnectorIcon'
import { SDK_FILTER_KEYS } from '../../lib/connection-icon'
import { packLaunch } from '../../lib/pack-status'

const INPUT_CLASS =
  'w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none'

/**
 * Install a connector package by reading its own manifest.
 *
 * The alternative is asking a person to transcribe a dozen field names —
 * `itemsPath`, `cursorArg`, `timestampField` — out of a README into the
 * generic MCP form, where a single typo produces a connection that silently
 * never fires. The connector already knows all of them, so it is asked.
 */
/**
 * Turn a connector's suggestions into the connection's own mapping.
 *
 * A suggestion, not a rule: this is the starting point the person setting up
 * the connection then owns, which is why it is written onto the connection
 * rather than read from the manifest every time.
 */
function seedStatusMapping(suggestions: SdkTrigger['statusMapping']): Record<string, TaskStatus> {
  const mapping: Record<string, TaskStatus> = {}
  for (const entry of suggestions ?? []) mapping[entry.upstream] = entry.suggestedLocal
  return mapping
}

export function SdkConnectorForm({
  onDone,
  onCancel,
  catalogEntry,
  pack
}: {
  onDone: () => void
  onCancel: () => void
  /**
   * Set when the install started from a connector Vorn already knows about.
   * The package name is then a detail of the catalog rather than something to
   * type, so the lookup step disappears and the form opens on the questions
   * only the person can answer.
   */
  catalogEntry?: ConnectorCatalogItem
  /** Set when installed as a pack, so the probe reads the files that will run. */
  pack?: InstalledConnectorPack
}) {
  const projects = useAppStore((s) => s.config?.projects || [])

  const [spec, setSpec] = useState('')
  const [probing, setProbing] = useState(false)
  const [manifest, setManifest] = useState<SdkConnectorManifest | null>(null)
  const [launch, setLaunch] = useState<{ command: string; args: string[] } | null>(null)
  const [triggerType, setTriggerType] = useState('')
  const [values, setValues] = useState<Record<string, string>>({})
  const [selectedProject, setSelectedProject] = useState(projects[0]?.name || '')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [identity, setIdentity] = useState<AuthProbeReport | null>(null)
  // A token stays one click away for the machine where the CLI is not the answer.
  const [useToken, setUseToken] = useState(false)

  /**
   * Read a connector by starting it.
   *
   * Takes the target rather than reading `spec`, so it closes over no state
   * and is safe to call from an effect.
   */
  const probe = useCallback(async (target: { command: string; args: string[] }) => {
    setError(null)
    setManifest(null)
    setProbing(true)
    try {
      const result = await window.api.probeSdkConnector(target)
      if (!result.ok) {
        setError(result.error)
        return
      }
      setManifest(result.manifest)
      setLaunch(target)
      setTriggerType(result.manifest.triggers[0]?.type ?? '')
      // Seed defaults so a field the user never touches still round-trips.
      setValues(Object.fromEntries(result.manifest.env.map((entry) => [entry.name, ''])))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setProbing(false)
    }
  }, [])

  // A catalog entry names its own package, so there is nothing to look up by
  // hand — go straight to reading it.
  //
  // Guarded because a probe spawns a child process that downloads a package:
  // React runs effects twice on mount in development, and two `npx` cold
  // installs race to fill the same form.
  const probedRef = useRef<string | null>(null)
  useEffect(() => {
    // The installed pack wins: it is the code that will run, and needs no registry.
    const target = pack ? packLaunch(pack) : catalogEntry?.launch
    const key = pack ? `${pack.id}@${pack.version}` : catalogEntry?.packageName
    if (!target || !key || probedRef.current === key) return
    probedRef.current = key
    void probe(target)
  }, [catalogEntry, pack, probe])

  const rung = manifest?.auth?.rung
  // Borrowing covers the credential, so its fields are hidden and cannot hold the form shut.
  const borrowing = rung === 'cli' && !useToken

  // Ask the tool who you are, once the manifest says there is a tool to ask.
  useEffect(() => {
    if (manifest?.auth?.rung !== 'cli') return
    let live = true
    void Promise.resolve().then(() => setIdentity(null))
    // A build that cannot ask has answered too; an identity is a courtesy, the connect still happens.
    void Promise.resolve(window.api.probeConnectorAuth?.(manifest.id)).then(
      (report) => live && setIdentity(report ?? { ok: null }),
      () => live && setIdentity({ ok: null })
    )
    return () => {
      live = false
    }
  }, [manifest])

  const trigger = manifest?.triggers.find((entry) => entry.type === triggerType)
  const borrowed = borrowing && manifest ? borrowableFromManifest(manifest.auth, manifest.env) : []
  const covered = new Set(borrowed.map((name) => name.toUpperCase()))
  // Nothing to fill in for a connector that asks for nothing; a secret the borrow covers is not asked for either.
  const fields = (manifest?.env ?? []).filter(
    (entry) => rung !== 'none' && !(entry.secret && covered.has(entry.name.toUpperCase()))
  )
  const missing = fields.filter((entry) => entry.required && !values[entry.name]?.trim())

  const handleSave = async () => {
    if (!manifest || !launch) return
    setError(null)
    setSaving(true)
    try {
      const plain: Record<string, string> = {}
      const secret: Record<string, string> = {}
      // Only what the form actually asked for.
      for (const entry of fields) {
        const value = values[entry.name]?.trim()
        if (!value) continue
        ;(entry.secret ? secret : plain)[entry.name] = value
      }

      const filters: Record<string, unknown> = {
        command: launch.command,
        args: JSON.stringify(launch.args),
        env: JSON.stringify(plain),
        // Recorded so the connection can be re-probed later without the user
        // retyping what they installed.
        [SDK_FILTER_KEYS.connectorId]: manifest.id,
        [SDK_FILTER_KEYS.version]: manifest.version,
        // Carried on the connection because a packaged connector is stored as
        // an `mcp` connection, so there is no connector id to key a glyph by.
        ...(manifest.icon && { [SDK_FILTER_KEYS.icon]: JSON.stringify(manifest.icon) })
      }

      if (Object.keys(secret).length > 0) {
        // Encrypted here, before it reaches the database, exactly as the
        // generic MCP form does for its own secret env blob.
        filters.secretEnv = await window.api.encryptString(JSON.stringify(secret))
      }

      // The whole point of the probe: these are the values a person would
      // otherwise have to copy by hand, and getting one wrong yields a
      // connection that polls and never fires.
      if (trigger) Object.assign(filters, trigger.filters)

      await window.api.createConnection({
        connectorId: 'mcp',
        name: trigger ? `${manifest.name}: ${trigger.label}` : manifest.name,
        filters,
        syncIntervalMinutes: 5,
        // Seeded from what the connector suggests. Left empty, every item it
        // ever imports lands as `todo` — a closed issue included.
        statusMapping: seedStatusMapping(trigger?.statusMapping),
        // Without this the connection is made and then sits silent: nothing
        // polls it until somebody builds the workflow by hand.
        ...(trigger?.defaultWorkflow && { seedWorkflow: trigger.defaultWorkflow }),
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
    <div className="space-y-3">
      {!catalogEntry && (
        <div>
          <label className="block text-xs text-gray-500 mb-1">Connector package</label>
          <div className="flex gap-2">
            <input
              type="text"
              value={spec}
              onChange={(e) => setSpec(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && spec.trim() && !probing) void probe(parseLaunchSpec(spec))
              }}
              placeholder="@vornrun/connector-kusto"
              className={INPUT_CLASS}
            />
            <button
              onClick={() => void probe(parseLaunchSpec(spec))}
              disabled={probing || !spec.trim()}
              className="px-3 py-1.5 text-sm bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50 flex items-center gap-1.5 shrink-0"
            >
              {probing ? <Loader2 size={12} className="animate-spin" /> : <Search size={12} />}
              {probing ? 'Reading…' : 'Look up'}
            </button>
          </div>
          <p className="text-[11px] text-gray-600 mt-1">
            An npm package name, or a command to run a connector from a local checkout.
          </p>
        </div>
      )}

      {probing && (
        <p className="text-[11px] text-gray-500">
          Downloading and starting the connector to read what it needs. This can take a moment the
          first time.
        </p>
      )}

      {manifest && (
        <>
          <div className="px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-sm">
            <div className="flex items-center gap-1.5 text-sm text-gray-200">
              {manifest.icon ? (
                <ConnectorIcon
                  connectorId="mcp"
                  icon={manifest.icon}
                  size={13}
                  className="text-gray-200 shrink-0"
                />
              ) : (
                <Check size={12} className="text-green-400 shrink-0" />
              )}
              <span className="font-medium">{manifest.name}</span>
              <span className="text-[11px] text-gray-500">v{manifest.version}</span>
            </div>
            {manifest.description && (
              <p className="text-[11px] text-gray-500 mt-1">{manifest.description}</p>
            )}
            {manifest.actions.length > 0 && (
              <p className="text-[11px] text-gray-600 mt-1">
                {manifest.actions.length} action{manifest.actions.length === 1 ? '' : 's'} available
                to workflow steps.
              </p>
            )}
          </div>

          {rung === 'none' && (
            <p className="text-[11px] text-gray-500">
              This connector asks for no sign-in — installing it was the whole setup, and its
              actions are ready to use in a workflow.
            </p>
          )}

          {manifest.triggers.length > 0 && rung !== 'none' && (
            <div>
              <label className="block text-xs text-gray-500 mb-1">Trigger</label>
              <select
                value={triggerType}
                onChange={(e) => setTriggerType(e.target.value)}
                className={INPUT_CLASS}
              >
                {manifest.triggers.map((entry) => (
                  <option key={entry.type} value={entry.type}>
                    {entry.label}
                  </option>
                ))}
              </select>
              {trigger?.description && (
                <p className="text-[11px] text-gray-600 mt-1">{trigger.description}</p>
              )}
            </div>
          )}

          {rung !== 'none' && (
            <div>
              <label className="block text-xs text-gray-500 mb-1">Project</label>
              <select
                value={selectedProject}
                onChange={(e) => setSelectedProject(e.target.value)}
                className={INPUT_CLASS}
              >
                {projects.map((project) => (
                  <option key={project.name} value={project.name}>
                    {project.name}
                  </option>
                ))}
              </select>
            </div>
          )}

          {rung === 'cli' && (
            <div className="px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-sm">
              {identity?.ok ? (
                <div className="flex items-center gap-1.5 text-sm text-gray-200">
                  <span className="w-1.5 h-1.5 rounded-full bg-status-sage shrink-0" />
                  <span>Signed in{identity.identity ? ` as ${identity.identity}` : ''}</span>
                  {manifest.auth?.probe && (
                    <span className="text-[11px] text-gray-500 font-mono">
                      · from {manifest.auth.probe.command}{' '}
                      {(manifest.auth.probe.args ?? []).join(' ')}
                    </span>
                  )}
                </div>
              ) : (
                <div className="text-[11px] text-gray-400">
                  {identity === null
                    ? 'Checking whether the tool it borrows is signed in…'
                    : // `ok: null` is an answer — this build cannot ask — and
                      // leaving the in-flight line up would read as a hang.
                      (identity.message ?? 'Nothing to check for this connector.')}
                  {identity?.installHint && (
                    <span className="block text-gray-600 mt-1">{identity.installHint}</span>
                  )}
                </div>
              )}
              {borrowed.length > 0 && (
                <p className="text-[11px] text-gray-600 mt-1">
                  Hands over {borrowed.join(', ')} from {manifest.auth?.probe?.command}.
                </p>
              )}
              <button
                onClick={() => {
                  // Whatever was typed into a field that is about to be hidden goes with it, so nothing invisible is carried into the save.
                  setValues((prev) => {
                    const kept = { ...prev }
                    for (const entry of manifest.env) if (entry.secret) delete kept[entry.name]
                    return kept
                  })
                  setUseToken((prev) => !prev)
                }}
                className="text-[11px] text-gray-500 hover:text-gray-300 transition-colors mt-1.5"
              >
                {useToken ? 'Borrow the signed-in tool instead' : 'Use a token instead'}
              </button>
            </div>
          )}

          {fields.map((entry) => (
            <div key={entry.name}>
              <label className="text-xs text-gray-500 mb-1 flex items-center gap-1.5">
                <span className="font-mono text-[11px]">{entry.name}</span>
                {entry.required && <span className="text-red-400">*</span>}
                {entry.secret && (
                  <span className="text-[9px] text-gray-600 uppercase tracking-wider">
                    · encrypted
                  </span>
                )}
              </label>
              <input
                type={entry.secret ? 'password' : 'text'}
                value={values[entry.name] ?? ''}
                onChange={(e) => setValues((prev) => ({ ...prev, [entry.name]: e.target.value }))}
                className={INPUT_CLASS}
              />
              {entry.description && (
                <p className="text-[11px] text-gray-600 mt-1">{entry.description}</p>
              )}
            </div>
          ))}
        </>
      )}

      {error && (
        <div className="text-[11px] text-red-400 flex items-start gap-1">
          <AlertCircle size={12} className="mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <div className="flex gap-2 pt-1">
        <button
          // A connector that asks for nothing was connected when it was installed; offering Connect again would make a second one.
          onClick={() => (rung === 'none' ? onDone() : void handleSave())}
          disabled={!manifest || saving || (rung !== 'none' && missing.length > 0)}
          className="px-4 py-1.5 text-sm bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50"
        >
          {rung === 'none' ? 'Done' : saving ? 'Connecting…' : 'Connect'}
        </button>
        <button
          onClick={onCancel}
          className="px-4 py-1.5 text-sm text-gray-500 hover:text-gray-300 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
