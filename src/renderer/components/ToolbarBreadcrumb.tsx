import { useRef } from 'react'
import { useAppStore } from '../stores'
import { MAIN_WORKTREE_SENTINEL } from '../stores/types'
import type { WorktreeInfo } from '../stores/types'
import { ChevronRight, GitBranch, ChevronDown } from 'lucide-react'
import { BranchPicker } from './BranchPicker'
import { useBranchSwitcher } from '../hooks/useBranchSwitcher'

const EMPTY_WORKTREES: WorktreeInfo[] = []

export function ToolbarBreadcrumb() {
  const activeProject = useAppStore((s) => s.activeProject)
  const setActiveGroup = useAppStore((s) => s.setActiveGroup)
  // Matched on the workspace too: a stale id is no selection anywhere else, and
  // a crumb naming it would be the only thing on screen that still believed it.
  const activeGroup = useAppStore((s) =>
    s.activeGroupId
      ? s.config?.sessionGroups?.find(
          (g) => g.id === s.activeGroupId && g.workspaceId === s.activeWorkspace
        )
      : undefined
  )
  const activeWorktreePath = useAppStore((s) => s.activeWorktreePath)
  const setActiveWorktreePath = useAppStore((s) => s.setActiveWorktreePath)
  const worktreeCache = useAppStore((s) => s.worktreeCache)
  const projects = useAppStore((s) => s.config?.projects)

  const project = projects?.find((p) => p.name === activeProject)
  const projectPath = project?.path
  const isMainWorktree = activeWorktreePath === MAIN_WORKTREE_SENTINEL

  const allWorktrees = projectPath
    ? (worktreeCache.get(projectPath) ?? EMPTY_WORKTREES)
    : EMPTY_WORKTREES
  const activeWorktree = isMainWorktree
    ? allWorktrees.find((wt) => wt.isMain)
    : allWorktrees.find((wt) => wt.path === activeWorktreePath)

  const worktreeName = isMainWorktree ? null : activeWorktree?.name
  const branchName = activeWorktree?.branch
  const branchCwd = isMainWorktree ? projectPath : (activeWorktreePath ?? undefined)

  const branchButtonRef = useRef<HTMLButtonElement>(null)
  const { showPicker, togglePicker, closePicker, isSwitching, selectBranch } = useBranchSwitcher({
    projectPath,
    branchCwd,
    branchName
  })

  // A group is the same selection slot as a project, so it gets the same crumb.
  if (!activeProject) {
    if (!activeGroup) return null
    return (
      <div className="flex items-center gap-0.5 text-[13px] min-w-0 max-w-[400px]">
        <button
          onClick={() => setActiveGroup(null)}
          className="text-white truncate hover:text-gray-300 transition-colors"
        >
          {activeGroup.name}
        </button>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-0.5 text-[13px] min-w-0 max-w-[400px]">
      {!activeWorktreePath ? (
        <span className="text-white truncate">{activeProject}</span>
      ) : (
        <>
          <button
            onClick={() => setActiveWorktreePath(null)}
            className="text-gray-500 hover:text-gray-300 transition-colors truncate"
          >
            {activeProject}
          </button>
          {worktreeName && (
            <>
              <ChevronRight size={10} className="text-gray-600 shrink-0 mx-0.5" />
              <span className="text-gray-400 truncate">{worktreeName}</span>
            </>
          )}
          {branchName && branchCwd && projectPath && (
            <>
              <ChevronRight size={10} className="text-gray-600 shrink-0 mx-0.5" />
              <div className="relative shrink-0">
                <button
                  ref={branchButtonRef}
                  onClick={togglePicker}
                  className={`flex items-center gap-1 transition-colors rounded px-1 -mx-1 ${
                    showPicker ? 'text-white bg-white/[0.08]' : 'text-white hover:bg-white/[0.06]'
                  } ${isSwitching ? 'opacity-50' : ''}`}
                >
                  <GitBranch size={11} className="text-gray-500 shrink-0" />
                  <span className="truncate max-w-[120px]">{branchName}</span>
                  <ChevronDown size={10} className="text-gray-500 shrink-0" />
                </button>
                {showPicker && (
                  <BranchPicker
                    projectPath={projectPath}
                    currentBranch={branchName}
                    onSelect={selectBranch}
                    onClose={closePicker}
                    anchorRef={branchButtonRef}
                  />
                )}
              </div>
            </>
          )}
        </>
      )}
    </div>
  )
}
