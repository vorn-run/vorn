import { useEffect } from 'react'
import { toast, type ToastAction } from './Toast'
import { removeWorktreeWithProgress } from '../lib/remove-worktree'

function basename(p: string): string {
  const trimmed = p.replace(/[\\/]+$/, '')
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  return idx === -1 ? trimmed : trimmed.slice(idx + 1)
}

export function WorktreeCleanupToastBridge() {
  useEffect(() => {
    const unsub = window.api.onWorktreeCleanup(({ projectPath, worktreePath }) => {
      const name = basename(worktreePath)
      const baseMessage = `Worktree "${name}" — last session ended`

      const buildActions = (force: boolean): ToastAction[] => [
        {
          label: 'Keep',
          onClick: (toastId) => toast.dismiss(toastId)
        },
        {
          label: 'Remove',
          tone: 'danger',
          onClick: (toastId) => {
            toast.dismiss(toastId)
            void removeWorktreeWithProgress({ projectPath, worktreePath, force })
          }
        }
      ]

      const id = toast(baseMessage, 'info', {
        duration: Number.POSITIVE_INFINITY,
        actions: buildActions(false)
      })

      // Background dirty check — update toast in place if uncommitted changes
      // detected. Use updateIfExists so a late-arriving result can't resurrect a
      // toast the user already dismissed via Keep/Remove/X.
      window.api
        .isWorktreeDirty(worktreePath)
        .then((dirty) => {
          if (!dirty) return
          toast.updateIfExists(id, `Worktree "${name}" has uncommitted changes`, 'warning', {
            duration: Number.POSITIVE_INFINITY,
            actions: buildActions(true)
          })
        })
        .catch(() => {
          toast.updateIfExists(
            id,
            `Worktree "${name}" — last session ended (changes check failed)`,
            'warning',
            {
              duration: Number.POSITIVE_INFINITY,
              actions: buildActions(true)
            }
          )
        })
    })
    return unsub
  }, [])

  return null
}
