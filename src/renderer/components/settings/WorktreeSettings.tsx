import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertTriangle,
  FolderGit2,
  GitBranch,
  HardDrive,
  Loader2,
  RefreshCw,
  Trash2
} from 'lucide-react'
import type {
  StaleBranch,
  WorktreeActionResult,
  WorktreeInventory,
  WorktreeInventoryEntry,
  WorktreeProjectInventory,
  WorktreeVerdictLevel
} from '../../../shared/types'
import { formatBytes } from '../../lib/format-bytes'
import { toast } from '../Toast'
import { withProgressToast } from '../../lib/progress-toast'
import { SettingsPageHeader } from './SettingsPageHeader'

/**
 * Separator for the `<projectPath> <branch>` keys that identify a stale branch
 * across projects. NUL can appear in neither a path nor a branch name.
 */
const KEY_SEP = '\u0000'

/** Which destructive action is showing its second, spelled-out confirm step. */
type Pending = 'reclaim' | 'remove' | null

const VERDICT_STYLES: Record<WorktreeVerdictLevel, { label: string; className: string }> = {
  keep: { label: 'Keep', className: 'text-gray-400 bg-white/[0.05]' },
  review: { label: 'Review', className: 'text-amber-300 bg-amber-500/[0.12]' },
  reclaim: { label: 'Build output only', className: 'text-blue-300 bg-blue-500/[0.12]' },
  remove: { label: 'Removable', className: 'text-blue-300 bg-blue-500/[0.12]' },
  orphan: { label: 'Orphan directory', className: 'text-red-300 bg-red-500/[0.12]' }
}

function isSelectable(entry: WorktreeInventoryEntry): boolean {
  return !entry.isMain && entry.activeSessionIds.length === 0
}

function idleLabel(entry: WorktreeInventoryEntry): string {
  if (entry.idleDays === null) return '—'
  if (entry.idleDays === 0) return 'today'
  return `${entry.idleDays}d idle`
}

/** Reports what a batch actually did, including the parts that failed. */
function reportResult(result: WorktreeActionResult | undefined, verb: string): void {
  if (!result) return
  const freed = result.freedBytes > 0 ? ` · freed ${formatBytes(result.freedBytes)}` : ''
  if (result.succeeded.length > 0) {
    toast(`${verb} ${result.succeeded.length}${freed}`, 'success')
  }
  for (const failure of result.failed) {
    toast(`${failure.path.split('/').pop()}: ${failure.error}`, 'error')
  }
}

export function WorktreeSettings() {
  const [inventory, setInventory] = useState<WorktreeInventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [selectedBranches, setSelectedBranches] = useState<Set<string>>(new Set())
  const [confirming, setConfirming] = useState<Pending>(null)
  const [busy, setBusy] = useState(false)
  const inFlight = useRef(false)

  // Nothing is set synchronously here: `loading` starts true for the first
  // scan, and the Rescan button flips it before calling in.
  const load = useCallback(async (refresh: boolean): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    try {
      const next = await window.api.getWorktreeInventory({ refresh })
      setInventory(next)
      // Drop selections for anything the rescan no longer reports.
      const live = new Set(next.projects.flatMap((p) => p.entries.map((e) => e.path)))
      setSelected((prev) => new Set([...prev].filter((p) => live.has(p))))
      const liveBranches = new Set(
        next.projects.flatMap((p) =>
          p.staleBranches.map((b) => `${p.projectPath}${KEY_SEP}${b.name}`)
        )
      )
      setSelectedBranches((prev) => new Set([...prev].filter((b) => liveBranches.has(b))))
    } catch (err) {
      toast(err instanceof Error ? err.message : 'Could not read worktrees', 'error')
    } finally {
      inFlight.current = false
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: scans worktrees via the main process on mount
    void load(false)
  }, [load])

  const allEntries = useMemo(() => inventory?.projects.flatMap((p) => p.entries) ?? [], [inventory])

  const totals = useMemo(() => {
    let onDisk = 0
    let artifacts = 0
    let removable = 0
    for (const entry of allEntries) {
      onDisk += entry.sizeBytes
      if (isSelectable(entry)) {
        artifacts += entry.artifactBytes
        if (entry.verdict.level === 'remove' || entry.verdict.level === 'orphan') {
          removable += entry.sizeBytes
        }
      }
    }
    return { onDisk, artifacts, removable }
  }, [allEntries])

  const selectedEntries = useMemo(
    () => allEntries.filter((e) => selected.has(e.path)),
    [allEntries, selected]
  )

  const selection = useMemo(() => {
    const orphans = selectedEntries.filter((e) => e.kind === 'orphan-dir')
    const worktrees = selectedEntries.filter((e) => e.kind === 'registered')
    return {
      orphans,
      worktrees,
      dirty: selectedEntries.filter((e) => e.isDirty),
      unmerged: worktrees.filter((e) => !e.isMerged),
      bytes: selectedEntries.reduce((sum, e) => sum + e.sizeBytes, 0),
      artifactBytes: selectedEntries.reduce((sum, e) => sum + e.artifactBytes, 0)
    }
  }, [selectedEntries])

  const reclaimTargets = useMemo(
    () => allEntries.filter((e) => isSelectable(e) && e.artifactBytes > 0),
    [allEntries]
  )

  const toggle = (path: string): void => {
    setConfirming(null)
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }

  const selectSuggested = (): void => {
    setConfirming(null)
    setSelected(
      new Set(allEntries.filter((e) => isSelectable(e) && e.verdict.autoSelect).map((e) => e.path))
    )
  }

  const rescan = (): void => {
    setLoading(true)
    void load(true)
  }

  const run = async (
    labels: { loading: string; success: string },
    fn: () => Promise<WorktreeActionResult | undefined>
  ): Promise<void> => {
    if (busy) return
    setBusy(true)
    setConfirming(null)
    const result = await withProgressToast(labels, async () => {
      const res = await fn()
      // A batch where every item failed is a failure, not a quiet success.
      if (res && res.succeeded.length === 0 && res.failed.length > 0) {
        throw new Error(res.failed[0].error)
      }
      return res
    })
    reportResult(result, labels.success)
    setBusy(false)
    rescan()
  }

  const handleReclaim = (): void =>
    void run({ loading: 'Deleting build output…', success: 'Reclaimed' }, () =>
      window.api.reclaimWorktreeArtifacts(reclaimTargets.map((e) => e.path))
    )

  const handleRemoveSelected = (): void =>
    void run({ loading: 'Removing worktrees…', success: 'Removed' }, async () => {
      const combined: WorktreeActionResult = {
        succeeded: [],
        failed: [],
        freedBytes: 0,
        deletedBranches: []
      }
      if (selection.worktrees.length > 0) {
        const res = await window.api.removeWorktrees(
          selection.worktrees.map((e) => ({
            projectPath: e.projectPath,
            worktreePath: e.path,
            force: e.isDirty,
            // Only vorn's own merged branches — an unmerged branch survives the
            // worktree so the commits stay reachable.
            deleteBranch: e.isMerged
          }))
        )
        combined.succeeded.push(...res.succeeded)
        combined.failed.push(...res.failed)
        combined.freedBytes += res.freedBytes
        combined.deletedBranches.push(...res.deletedBranches)
      }
      if (selection.orphans.length > 0) {
        const res = await window.api.pruneOrphanWorktrees(selection.orphans.map((e) => e.path))
        combined.succeeded.push(...res.succeeded)
        combined.failed.push(...res.failed)
        combined.freedBytes += res.freedBytes
      }
      return combined
    })

  const handleDeleteBranches = (project: WorktreeProjectInventory): void => {
    const branches = project.staleBranches
      .filter((b) => selectedBranches.has(`${project.projectPath}${KEY_SEP}${b.name}`))
      .map((b) => b.name)
    if (branches.length === 0) return
    void run({ loading: 'Deleting branches…', success: 'Deleted' }, async () => {
      const res = await window.api.deleteBranches(project.projectPath, branches, false)
      return {
        succeeded: res.deleted,
        failed: res.failed.map((f) => ({ path: f.branch, error: f.error })),
        freedBytes: 0,
        deletedBranches: res.deleted
      }
    })
  }

  const projects = inventory?.projects ?? []
  const hasAnything = projects.some((p) => p.entries.length > 0 || p.staleBranches.length > 0)

  return (
    <div className="pb-24">
      <SettingsPageHeader
        title="Worktrees"
        description="What every worktree costs on disk, and what can safely go"
        actions={
          <button
            onClick={rescan}
            disabled={loading || busy}
            className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] text-gray-300 bg-white/[0.04]
                       hover:bg-white/[0.08] rounded-lg transition-colors disabled:opacity-50"
          >
            {loading ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
            Rescan
          </button>
        }
      />

      <SummaryCard
        onDisk={totals.onDisk}
        artifacts={totals.artifacts}
        removable={totals.removable}
        loading={loading}
        busy={busy}
        confirming={confirming === 'reclaim'}
        targetCount={reclaimTargets.length}
        onAsk={() => setConfirming(confirming === 'reclaim' ? null : 'reclaim')}
        onConfirm={handleReclaim}
      />

      {loading && !inventory && (
        <div className="flex items-center gap-2 py-10 text-[13px] text-gray-500 justify-center">
          <Loader2 size={15} className="animate-spin" />
          Measuring worktrees…
        </div>
      )}

      {!loading && !hasAnything && (
        <div className="py-10 text-center text-[13px] text-gray-500">
          No worktrees yet. They appear here as soon as you create one.
        </div>
      )}

      {projects.map((project) => (
        <ProjectGroup
          key={`${project.projectPath}:${project.remoteHostId ?? 'local'}`}
          project={project}
          selected={selected}
          onToggle={toggle}
          selectedBranches={selectedBranches}
          onToggleBranch={(name) =>
            setSelectedBranches((prev) => {
              const key = `${project.projectPath}${KEY_SEP}${name}`
              const next = new Set(prev)
              if (next.has(key)) next.delete(key)
              else next.add(key)
              return next
            })
          }
          onDeleteBranches={() => handleDeleteBranches(project)}
          busy={busy}
        />
      ))}

      {allEntries.some((e) => isSelectable(e) && e.verdict.autoSelect) && (
        <button
          onClick={selectSuggested}
          className="mt-4 text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
        >
          Select everything merged and idle
        </button>
      )}

      <AnimatePresence>
        {selectedEntries.length > 0 && (
          <SelectionBar
            count={selectedEntries.length}
            bytes={selection.bytes}
            dirtyCount={selection.dirty.length}
            unmergedCount={selection.unmerged.length}
            orphanCount={selection.orphans.length}
            confirming={confirming === 'remove'}
            busy={busy}
            onClear={() => {
              setSelected(new Set())
              setConfirming(null)
            }}
            onAsk={() => setConfirming(confirming === 'remove' ? null : 'remove')}
            onConfirm={handleRemoveSelected}
          />
        )}
      </AnimatePresence>
    </div>
  )
}

// ─── Summary ────────────────────────────────────────────────────

function SummaryCard({
  onDisk,
  artifacts,
  removable,
  loading,
  busy,
  confirming,
  targetCount,
  onAsk,
  onConfirm
}: {
  onDisk: number
  artifacts: number
  removable: number
  loading: boolean
  busy: boolean
  confirming: boolean
  targetCount: number
  onAsk: () => void
  onConfirm: () => void
}) {
  const artifactPct = onDisk > 0 ? Math.min(100, (artifacts / onDisk) * 100) : 0
  const removablePct = onDisk > 0 ? Math.min(100 - artifactPct, (removable / onDisk) * 100) : 0

  return (
    <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-5 mb-6">
      <div className="flex items-end justify-between gap-6 flex-wrap">
        <div>
          <div className="flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-gray-500">
            <HardDrive size={12} />
            On disk
          </div>
          <div className="text-[30px] leading-[38px] font-semibold text-white tabular-nums mt-1">
            {formatBytes(onDisk)}
          </div>
        </div>
        <div className="text-right">
          <div className="text-[11px] uppercase tracking-wider text-gray-500">
            Build output — rebuilt by a reinstall
          </div>
          <div className="text-[20px] leading-[28px] font-semibold text-blue-300 tabular-nums mt-1">
            {formatBytes(artifacts)}
          </div>
        </div>
      </div>

      <div className="mt-4 h-2 rounded-full bg-white/[0.06] overflow-hidden flex">
        <div className="h-full bg-blue-400/70" style={{ width: `${artifactPct}%` }} />
        <div className="h-full bg-blue-400/25" style={{ width: `${removablePct}%` }} />
      </div>
      <div className="mt-2 flex gap-4 text-[11px] text-gray-500">
        <span>Build output</span>
        <span>Removable worktrees</span>
        <span className="ml-auto tabular-nums">{formatBytes(removable)} in merged worktrees</span>
      </div>

      {artifacts > 0 && (
        <div className="mt-4 flex items-center gap-2">
          <button
            onClick={confirming ? onConfirm : onAsk}
            disabled={loading || busy}
            className={`flex items-center gap-2 px-3 py-2 text-[11px] rounded-md transition-colors
                        disabled:opacity-50 ${
                          confirming
                            ? 'text-white bg-blue-500/25 hover:bg-blue-500/35 border border-blue-500/30'
                            : 'text-blue-300 bg-blue-500/[0.12] hover:bg-blue-500/20'
                        }`}
          >
            <Trash2 size={13} />
            {confirming
              ? `Delete build output in ${targetCount} worktree${targetCount === 1 ? '' : 's'} — frees ${formatBytes(artifacts)}`
              : `Reclaim ${formatBytes(artifacts)} of build output`}
          </button>
          {confirming && (
            <button
              onClick={onAsk}
              className="px-3 py-2 text-[11px] text-gray-400 hover:text-white transition-colors"
            >
              Cancel
            </button>
          )}
        </div>
      )}
    </div>
  )
}

// ─── Project group ──────────────────────────────────────────────

function ProjectGroup({
  project,
  selected,
  onToggle,
  selectedBranches,
  onToggleBranch,
  onDeleteBranches,
  busy
}: {
  project: WorktreeProjectInventory
  selected: Set<string>
  onToggle: (path: string) => void
  selectedBranches: Set<string>
  onToggleBranch: (name: string) => void
  onDeleteBranches: () => void
  busy: boolean
}) {
  const entries = [...project.entries]
    .filter((e) => !e.isMain)
    .sort((a, b) => b.sizeBytes - a.sizeBytes)
  const total = project.entries.reduce((sum, e) => sum + e.sizeBytes, 0)

  if (entries.length === 0 && project.staleBranches.length === 0 && !project.error) return null

  return (
    <div className="mb-6">
      <div className="flex items-baseline justify-between gap-3 pb-2 border-b border-white/[0.06]">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="text-[13px] font-medium text-gray-200 truncate">
            {project.projectName}
          </span>
          {project.defaultBranch && (
            <span className="text-[11px] text-gray-600 font-mono shrink-0">
              vs {project.defaultBranch}
            </span>
          )}
          {project.remoteHostId && (
            <span className="text-[11px] text-gray-600 shrink-0">remote</span>
          )}
        </div>
        <span className="text-[11px] text-gray-500 tabular-nums shrink-0">
          {formatBytes(total)}
        </span>
      </div>

      {project.error && (
        <div className="py-3 text-[11px] text-gray-500">Could not scan — {project.error}</div>
      )}

      <div>
        {entries.map((entry) => (
          <WorktreeRow
            key={entry.path}
            entry={entry}
            checked={selected.has(entry.path)}
            onToggle={() => onToggle(entry.path)}
          />
        ))}
      </div>

      {project.staleBranches.length > 0 && (
        <StaleBranchStrip
          projectPath={project.projectPath}
          branches={project.staleBranches}
          selected={selectedBranches}
          onToggle={onToggleBranch}
          onDelete={onDeleteBranches}
          busy={busy}
        />
      )}
    </div>
  )
}

function WorktreeRow({
  entry,
  checked,
  onToggle
}: {
  entry: WorktreeInventoryEntry
  checked: boolean
  onToggle: () => void
}) {
  const selectable = isSelectable(entry)
  const style = VERDICT_STYLES[entry.verdict.level]
  const artifactPct = entry.sizeBytes > 0 ? (entry.artifactBytes / entry.sizeBytes) * 100 : 0

  return (
    <div
      className={`flex items-center gap-3 py-2.5 border-b border-white/[0.04] last:border-b-0 ${
        selectable ? '' : 'opacity-50'
      }`}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={!selectable}
        onChange={onToggle}
        aria-label={`Select ${entry.name}`}
        className="shrink-0 accent-blue-500 disabled:cursor-not-allowed"
      />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 min-w-0">
          <FolderGit2 size={13} className="text-gray-600 shrink-0" />
          <span className="text-[13px] text-gray-200 truncate">{entry.name}</span>
          <span className={`text-[10px] px-1.5 py-0.5 rounded shrink-0 ${style.className}`}>
            {style.label}
          </span>
        </div>
        <div className="flex items-center gap-2 mt-1 text-[11px] text-gray-600 min-w-0">
          <span className="font-mono truncate">{entry.branch ?? 'no branch'}</span>
          <span className="shrink-0">·</span>
          <span className="shrink-0">{idleLabel(entry)}</span>
          {entry.verdict.reasons.length > 0 && (
            <>
              <span className="shrink-0">·</span>
              <span className="truncate">{entry.verdict.reasons.join(', ')}</span>
            </>
          )}
        </div>
      </div>

      <div className="shrink-0 w-[86px] text-right">
        <div className="text-[11px] text-gray-300 tabular-nums">
          {entry.sizeMeasured ? formatBytes(entry.sizeBytes) : '—'}
        </div>
        {entry.artifactBytes > 0 && (
          <div className="mt-1 h-1 rounded-full bg-white/[0.06] overflow-hidden">
            <div className="h-full bg-blue-400/60" style={{ width: `${artifactPct}%` }} />
          </div>
        )}
      </div>
    </div>
  )
}

// ─── Stale branches ─────────────────────────────────────────────

function StaleBranchStrip({
  projectPath,
  branches,
  selected,
  onToggle,
  onDelete,
  busy
}: {
  projectPath: string
  branches: StaleBranch[]
  selected: Set<string>
  onToggle: (name: string) => void
  onDelete: () => void
  busy: boolean
}) {
  const [expanded, setExpanded] = useState(false)
  const merged = branches.filter((b) => b.isMerged)
  const chosen = branches.filter((b) => selected.has(`${projectPath}${KEY_SEP}${b.name}`))

  return (
    <div className="mt-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2.5">
      <div className="flex items-center gap-2 flex-wrap">
        <GitBranch size={13} className="text-gray-600 shrink-0" />
        <span className="text-[11px] text-gray-400">
          {branches.length} branch{branches.length === 1 ? '' : 'es'} left behind by removed
          worktrees
          {merged.length > 0 && ` — ${merged.length} already merged`}
        </span>
        <button
          onClick={() => setExpanded(!expanded)}
          className="ml-auto text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
        >
          {expanded ? 'Hide' : 'Show'}
        </button>
      </div>

      {expanded && (
        <>
          <div className="mt-2.5 flex flex-wrap gap-1.5">
            {branches.map((branch) => {
              const key = `${projectPath}${KEY_SEP}${branch.name}`
              const isSelected = selected.has(key)
              return (
                <button
                  key={branch.name}
                  onClick={() => onToggle(branch.name)}
                  title={
                    branch.isMerged
                      ? 'Merged — deleting loses nothing'
                      : 'Not merged — git will refuse to delete it'
                  }
                  className={`font-mono text-[11px] px-2 py-1 rounded border transition-colors ${
                    isSelected
                      ? 'border-red-400/40 bg-red-500/[0.15] text-red-200'
                      : branch.isMerged
                        ? 'border-white/[0.08] bg-white/[0.03] text-gray-400 hover:text-white'
                        : 'border-amber-500/25 bg-amber-500/[0.06] text-amber-300/80 hover:text-amber-200'
                  }`}
                >
                  {branch.name}
                </button>
              )
            })}
          </div>

          <div className="mt-3 flex items-center gap-2">
            <button
              onClick={() =>
                merged.forEach((b) => {
                  if (!selected.has(`${projectPath}${KEY_SEP}${b.name}`)) onToggle(b.name)
                })
              }
              disabled={merged.length === 0}
              className="text-[11px] text-gray-500 hover:text-gray-300 transition-colors disabled:opacity-40"
            >
              Select {merged.length} merged
            </button>
            <button
              onClick={onDelete}
              disabled={chosen.length === 0 || busy}
              className="ml-auto flex items-center gap-1.5 px-2.5 py-1 text-[11px] text-red-300
                         bg-red-500/[0.1] hover:bg-red-500/20 rounded-md transition-colors
                         disabled:opacity-40"
            >
              <Trash2 size={11} />
              Delete {chosen.length || ''} branch{chosen.length === 1 ? '' : 'es'}
            </button>
          </div>
          <p className="mt-2 text-[10px] text-gray-600">
            Deletion uses <span className="font-mono">git branch -d</span>, so git refuses anything
            that isn&apos;t merged.
          </p>
        </>
      )}
    </div>
  )
}

// ─── Selection bar ──────────────────────────────────────────────

function SelectionBar({
  count,
  bytes,
  dirtyCount,
  unmergedCount,
  orphanCount,
  confirming,
  busy,
  onClear,
  onAsk,
  onConfirm
}: {
  count: number
  bytes: number
  dirtyCount: number
  unmergedCount: number
  orphanCount: number
  confirming: boolean
  busy: boolean
  onClear: () => void
  onAsk: () => void
  onConfirm: () => void
}) {
  const warnings: string[] = []
  if (dirtyCount > 0) {
    warnings.push(
      `${dirtyCount} ${dirtyCount === 1 ? 'has' : 'have'} uncommitted changes that will be lost`
    )
  }
  if (unmergedCount > 0) {
    warnings.push(`${unmergedCount} not merged — the branch is kept`)
  }
  if (orphanCount > 0) {
    warnings.push(`${orphanCount} deleted from disk directly`)
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 12 }}
      transition={{ type: 'spring', stiffness: 400, damping: 32 }}
      className="fixed bottom-6 left-1/2 -translate-x-1/2 z-10 w-[min(560px,calc(100vw-4rem))]
                 rounded-xl border border-white/[0.1] shadow-2xl px-4 py-3 bg-surface-overlay"
    >
      <div className="flex items-center gap-3">
        <div className="min-w-0">
          <div className="text-[13px] text-white">
            {count} selected ·{' '}
            <span className="tabular-nums text-gray-300">frees {formatBytes(bytes)}</span>
          </div>
          {warnings.length > 0 && (
            <div className="flex items-start gap-1.5 mt-1">
              <AlertTriangle size={11} className="text-amber-400 shrink-0 mt-0.5" />
              <span className="text-[11px] text-amber-300/85">{warnings.join(' · ')}</span>
            </div>
          )}
        </div>

        <div className="ml-auto flex items-center gap-2 shrink-0">
          <button
            onClick={onClear}
            className="px-2.5 py-1.5 text-[11px] text-gray-400 hover:text-white transition-colors"
          >
            Clear
          </button>
          <button
            onClick={confirming ? onConfirm : onAsk}
            disabled={busy}
            className={`flex items-center gap-1.5 px-3 py-1.5 text-[11px] rounded-md transition-colors
                        disabled:opacity-50 ${
                          confirming
                            ? 'text-white bg-red-500/30 hover:bg-red-500/45 border border-red-400/40'
                            : 'text-red-300 bg-red-500/[0.12] hover:bg-red-500/20'
                        }`}
          >
            <Trash2 size={12} />
            {confirming ? `Yes — remove ${count} and free ${formatBytes(bytes)}` : 'Remove'}
          </button>
        </div>
      </div>
      {confirming && <p className="mt-2 text-[11px] text-gray-500">This cannot be undone.</p>}
    </motion.div>
  )
}
