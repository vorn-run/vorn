import { useState, useRef } from 'react'
import { FolderGit2, GitBranch, ChevronRight, Plus, Pencil, Trash2, Terminal } from 'lucide-react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { InlineRename } from '../InlineRename'
import { toast } from '../Toast'
import { withProgressToast } from '../../lib/progress-toast'
import { createSessionFromProject, createShellInProject } from '../../lib/session-utils'
import { requestWorktreeDelete } from '../WorktreeCleanupDialog'
import type { WorktreeInfo } from '../../stores/types'

export function WorktreeItem({
  worktree,
  projectPath,
  projectName,
  isActiveWorktree,
  sessionCount,
  onSelect,
  onWorktreesChanged,
  sessionsExpanded,
  onToggleSessionsExpanded
}: {
  worktree: WorktreeInfo
  projectPath: string
  projectName: string
  isActiveWorktree: boolean
  sessionCount: number
  onSelect: () => void
  onWorktreesChanged: () => void
  sessionsExpanded?: boolean
  onToggleSessionsExpanded?: () => void
}) {
  const config = useAppStore((s) => s.config)
  const [renaming, setRenaming] = useState(false)
  const [renameValue, setRenameValue] = useState('')
  const [creatingSession, setCreatingSession] = useState(false)
  const [creatingTerminal, setCreatingTerminal] = useState(false)
  const [removing, setRemoving] = useState(false)
  const creatingSessionLock = useRef(false)
  const creatingTerminalLock = useRef(false)
  const removingLock = useRef(false)

  const wt = worktree

  if (renaming) {
    const commitRename = async (trimmed: string): Promise<void> => {
      if (trimmed && trimmed !== wt.name) {
        const result = await window.api.renameWorktree(wt.path, trimmed)
        if (result) {
          toast.success('Worktree renamed')
          onWorktreesChanged()
        } else {
          toast.error('Failed to rename worktree')
        }
      }
      setRenaming(false)
    }

    return (
      <div className="group/wt flex items-center gap-2 px-2 py-1.5 min-w-0">
        <FolderGit2 size={14} className="text-gray-500 shrink-0" strokeWidth={1.5} />
        <InlineRename
          value={renameValue}
          onCommit={(next) => void commitRename(next)}
          onCancel={() => setRenaming(false)}
          className="flex-1 min-w-0 text-[12px]"
        />
      </div>
    )
  }

  return (
    <div className="group/wt relative flex items-center">
      {isActiveWorktree && (
        <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full z-10" />
      )}
      {onToggleSessionsExpanded ? (
        <span
          role="button"
          tabIndex={0}
          aria-expanded={sessionsExpanded}
          aria-label="Toggle sessions"
          className="relative w-[14px] h-[14px] shrink-0 ml-2 cursor-pointer"
          onClick={(e) => {
            e.stopPropagation()
            onToggleSessionsExpanded()
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.stopPropagation()
              e.preventDefault()
              onToggleSessionsExpanded()
            }
          }}
        >
          <span className="group-hover/wt:hidden flex items-center justify-center w-full h-full">
            <FolderGit2 size={14} className="text-gray-500" strokeWidth={1.5} />
          </span>
          <ChevronRight
            size={12}
            strokeWidth={2.5}
            className={`hidden group-hover/wt:block text-gray-500 transition-transform absolute top-[1px] left-[1px] ${sessionsExpanded ? 'rotate-90' : ''}`}
          />
        </span>
      ) : (
        <FolderGit2 size={14} className="text-gray-500 shrink-0 ml-2" strokeWidth={1.5} />
      )}
      <button
        onClick={onSelect}
        className={`flex-1 text-left pl-2 pr-2 py-1.5 rounded-md text-[13px] flex items-center gap-2 min-w-0 transition-colors ${
          isActiveWorktree ? 'text-white' : 'text-gray-400 hover:text-white hover:bg-white/[0.04]'
        }`}
      >
        <div className="flex flex-col min-w-0 flex-1">
          <span className="truncate">{wt.name}</span>
          <span className="text-[10px] text-gray-600 flex items-center gap-1 truncate">
            <GitBranch size={8} className="shrink-0" />
            {wt.branch}
          </span>
        </div>
        {sessionCount > 0 && (
          <span className="text-gray-600 text-[11px] ml-auto group-hover/wt:hidden shrink-0">
            {sessionCount}
          </span>
        )}
      </button>
      <div className="hidden group-hover/wt:flex items-center gap-0.5 ml-auto pr-2 shrink-0">
        <Tooltip label="New terminal" position="right">
          <button
            type="button"
            aria-label="New terminal"
            disabled={creatingTerminal}
            onClick={(e) => {
              e.stopPropagation()
              if (creatingTerminalLock.current) return
              creatingTerminalLock.current = true
              setCreatingTerminal(true)
              const proj = config?.projects.find((p) => p.name === projectName)
              void createShellInProject(wt.path, {
                project: proj,
                worktreePath: wt.path,
                worktreeName: wt.name,
                branch: wt.branch
              }).finally(() => {
                creatingTerminalLock.current = false
                setCreatingTerminal(false)
              })
            }}
            className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
          >
            <Terminal size={14} strokeWidth={1.5} />
          </button>
        </Tooltip>
        <Tooltip label="New session" position="right">
          <button
            type="button"
            aria-label="New session"
            disabled={creatingSession}
            onClick={(e) => {
              e.stopPropagation()
              if (creatingSessionLock.current) return
              creatingSessionLock.current = true
              setCreatingSession(true)
              void withProgressToast(
                { loading: 'Starting session…', success: 'Session started' },
                async () => {
                  const proj = config?.projects.find((p) => p.name === projectName)
                  if (!proj) throw new Error(`Project "${projectName}" not found`)
                  await createSessionFromProject(proj, {
                    branch: wt.branch,
                    existingWorktreePath: wt.path
                  })
                }
              ).finally(() => {
                creatingSessionLock.current = false
                setCreatingSession(false)
              })
            }}
            className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
          >
            <Plus size={14} strokeWidth={2} />
          </button>
        </Tooltip>
        <Tooltip label="Rename worktree" position="right">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              setRenaming(true)
              setRenameValue(wt.name)
            }}
            className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors"
          >
            <Pencil size={14} strokeWidth={2} />
          </button>
        </Tooltip>
        <Tooltip label="Remove worktree" position="right">
          <button
            type="button"
            disabled={removing}
            onClick={async (e) => {
              e.stopPropagation()
              if (removingLock.current) return
              removingLock.current = true
              try {
                const { count, sessionIds } = await window.api.getWorktreeActiveSessions(wt.path)
                if (count > 0 || wt.isDirty) {
                  requestWorktreeDelete({
                    projectPath,
                    worktreePath: wt.path,
                    sessionIds
                  })
                  removingLock.current = false
                } else {
                  setRemoving(true)
                  void withProgressToast(
                    { loading: 'Removing worktree…', success: 'Worktree removed' },
                    async () => {
                      const removed = await window.api.removeWorktree(projectPath, wt.path, false)
                      if (!removed) throw new Error('Failed to remove worktree')
                      onWorktreesChanged()
                    }
                  ).finally(() => {
                    removingLock.current = false
                    setRemoving(false)
                  })
                }
              } catch (err) {
                removingLock.current = false
                toast.error(err instanceof Error ? err.message : 'Failed to check worktree')
              }
            }}
            className="text-gray-500 hover:text-red-400 p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
          >
            <Trash2 size={14} strokeWidth={2} />
          </button>
        </Tooltip>
      </div>
    </div>
  )
}
