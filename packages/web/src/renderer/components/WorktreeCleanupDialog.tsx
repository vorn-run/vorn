import { useState, useEffect, useRef, useCallback } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { FolderGit2, Trash2, AlertTriangle } from 'lucide-react'
import { removeWorktreeWithProgress } from '../lib/remove-worktree'

interface ExplicitDeleteInfo {
  projectPath: string
  worktreePath: string
  sessionIds: string[]
}

type DirtyState = 'checking' | 'clean' | 'dirty' | 'unknown'

type ExplicitDeleteCallback = (info: ExplicitDeleteInfo) => void
const subscribers = new Set<ExplicitDeleteCallback>()

// eslint-disable-next-line react-refresh/only-export-components -- lightweight pub/sub trigger, not a component
export function requestWorktreeDelete(info: ExplicitDeleteInfo): void {
  for (const cb of subscribers) cb(info)
}

function useExplicitDeleteSubscription(cb: ExplicitDeleteCallback): void {
  const cbRef = useRef(cb)
  useEffect(() => {
    cbRef.current = cb
  })
  useEffect(() => {
    const handler: ExplicitDeleteCallback = (info) => cbRef.current(info)
    subscribers.add(handler)
    return () => {
      subscribers.delete(handler)
    }
  }, [])
}

export function WorktreeCleanupDialog() {
  const [info, setInfo] = useState<ExplicitDeleteInfo | null>(null)
  const [dirtyState, setDirtyState] = useState<DirtyState>('checking')
  const checkIdRef = useRef(0)
  const removingLock = useRef(false)

  const isVisible = info !== null

  useExplicitDeleteSubscription(
    useCallback((next: ExplicitDeleteInfo) => {
      setInfo(next)
      // Reset the re-entry guard so the new dialog can act even if a prior
      // removal is still completing in the background.
      removingLock.current = false
      const id = ++checkIdRef.current
      setDirtyState('checking')
      window.api
        .isWorktreeDirty(next.worktreePath)
        .then((dirty) => {
          if (checkIdRef.current === id) setDirtyState(dirty ? 'dirty' : 'clean')
        })
        .catch(() => {
          if (checkIdRef.current === id) setDirtyState('unknown')
        })
    }, [])
  )

  const handleClose = (): void => {
    setInfo(null)
  }

  const handleRemove = (): void => {
    if (!info) return
    if (removingLock.current) return
    removingLock.current = true
    const force = dirtyState === 'dirty' || dirtyState === 'unknown'
    const { projectPath, worktreePath, sessionIds } = info
    handleClose()

    void removeWorktreeWithProgress({
      projectPath,
      worktreePath,
      sessionIds,
      force
    }).finally(() => {
      removingLock.current = false
    })
  }

  const showWarning = dirtyState === 'dirty' || dirtyState === 'unknown'
  const sessionCount = info?.sessionIds.length ?? 0

  return (
    <AnimatePresence>
      {isVisible && info && (
        <>
          <motion.div
            className="fixed inset-0 bg-black/40 z-[60]"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={handleClose}
          />
          <motion.div
            className="fixed top-1/2 left-1/2 z-[60] w-[420px] border border-white/[0.08]
                       rounded-xl shadow-2xl overflow-hidden bg-surface-raised"
            initial={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
            exit={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          >
            <div className="px-5 py-4 border-b border-white/[0.06] flex items-center gap-3">
              <FolderGit2 size={18} className="text-amber-400 shrink-0" />
              <div>
                <h3 className="text-[13px] font-medium text-white">Remove Worktree</h3>
                <p className="text-[11px] text-gray-500 mt-0.5">
                  {sessionCount > 0
                    ? 'This worktree has active sessions'
                    : 'Remove this worktree from disk?'}
                </p>
              </div>
            </div>

            <div className="px-5 py-3">
              <div className="px-3 py-2 bg-white/[0.03] border border-white/[0.06] rounded-lg">
                <p className="text-[11px] text-gray-500 font-mono truncate">{info.worktreePath}</p>
              </div>
            </div>

            {sessionCount > 0 && (
              <div className="mx-5 mb-2 px-3 py-2 bg-amber-500/[0.08] border border-amber-500/20 rounded-lg flex items-start gap-2">
                <AlertTriangle size={14} className="text-amber-400 shrink-0 mt-0.5" />
                <p className="text-[11px] text-amber-300/90">
                  {sessionCount} session{sessionCount > 1 ? 's are' : ' is'} running in this
                  worktree. {sessionCount > 1 ? 'They' : 'It'} will be terminated.
                </p>
              </div>
            )}

            {dirtyState !== 'checking' && showWarning && (
              <div className="mx-5 mb-2 px-3 py-2 bg-amber-500/[0.08] border border-amber-500/20 rounded-lg flex items-start gap-2">
                <AlertTriangle size={14} className="text-amber-400 shrink-0 mt-0.5" />
                <p className="text-[11px] text-amber-300/90">
                  {dirtyState === 'dirty'
                    ? 'This worktree has uncommitted changes that will be permanently lost.'
                    : 'Unable to check for uncommitted changes. Removal will use --force.'}
                </p>
              </div>
            )}

            <div className="px-5 py-3 border-t border-white/[0.06] flex justify-end gap-2">
              <button
                onClick={handleClose}
                className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] text-gray-300
                           bg-white/[0.04] hover:bg-white/[0.08] rounded-md transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleRemove}
                disabled={dirtyState === 'checking'}
                className={`flex items-center gap-1.5 px-3 py-1.5 text-[11px] rounded-md transition-colors
                           disabled:opacity-50 ${
                             showWarning || sessionCount > 0
                               ? 'text-red-300 bg-red-500/[0.15] hover:bg-red-500/[0.25] border border-red-500/30'
                               : 'text-red-400 bg-red-500/[0.08] hover:bg-red-500/[0.15]'
                           }`}
              >
                <Trash2 size={12} />
                {dirtyState === 'checking'
                  ? 'Checking…'
                  : sessionCount > 0
                    ? 'Close sessions & remove'
                    : showWarning
                      ? 'Remove anyway'
                      : 'Remove'}
              </button>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
