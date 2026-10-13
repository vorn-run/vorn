import { useState } from 'react'
import { motion } from 'framer-motion'
import { X, GitCommitHorizontal, ArrowUp, Check, Loader2, GitBranch } from 'lucide-react'
import { GitDiffStat } from '../../shared/types'
import { toast } from './Toast'

type CommitAction = 'commit' | 'commit-push'

interface Props {
  cwd: string
  branch: string | undefined
  stat: GitDiffStat | undefined
  onClose: () => void
  onCommitted: () => void
}

export function CommitDialog({ cwd, branch, stat, onClose, onCommitted }: Props) {
  const [message, setMessage] = useState('')
  const [includeUnstaged, setIncludeUnstaged] = useState(true)
  const [action, setAction] = useState<CommitAction>('commit')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const canCommit = message.trim().length > 0 && !loading

  const handleContinue = async (): Promise<void> => {
    if (!canCommit) return
    setLoading(true)
    setError(null)

    try {
      const result = await window.api.gitCommit({
        cwd,
        message: message.trim(),
        includeUnstaged
      })

      if (!result.success) {
        setError(result.error || 'Commit failed')
        setLoading(false)
        return
      }

      if (action === 'commit-push') {
        const pushResult = await window.api.gitPush(cwd)
        if (!pushResult.success) {
          setError(`Committed, but push failed: ${pushResult.error}`)
          setLoading(false)
          return
        }
      }

      toast.success(action === 'commit-push' ? 'Committed and pushed' : 'Changes committed')
      onCommitted()
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setLoading(false)
    }
  }

  const actions: { id: CommitAction; label: string; icon: typeof GitCommitHorizontal }[] = [
    { id: 'commit', label: 'Commit', icon: GitCommitHorizontal },
    { id: 'commit-push', label: 'Commit and push', icon: ArrowUp }
  ]

  return (
    <>
      {/* Backdrop */}
      <motion.div
        className="fixed inset-0 z-[60]"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        onClick={onClose}
        style={{ background: 'rgba(0, 0, 0, 0.5)' }}
      />

      {/* Dialog */}
      <motion.div
        className="fixed z-[70] rounded-xl border border-white/[0.08] shadow-2xl w-[440px]"
        style={{
          background: 'var(--color-surface-overlay)',
          top: '50%',
          left: '50%',
          x: '-50%',
          y: '-50%'
        }}
        initial={{ opacity: 0, scale: 0.97 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      >
        {/* Header */}
        <div className="flex items-center gap-3 px-5 pt-5 pb-3">
          <div className="p-2 rounded-lg bg-white/[0.06]">
            <GitCommitHorizontal size={18} className="text-gray-300" strokeWidth={1.5} />
          </div>
          <span className="flex-1 text-[18px] font-semibold text-gray-100">
            Commit your changes
          </span>
          <button
            onClick={onClose}
            className="p-1 text-gray-400 hover:text-white rounded transition-colors"
          >
            <X size={18} strokeWidth={1.5} />
          </button>
        </div>

        <div className="px-5 pb-5 space-y-4">
          {/* Branch */}
          <div className="flex items-center justify-between">
            <span className="text-[14px] text-gray-300 font-medium">Branch</span>
            <span className="flex items-center gap-1.5 text-[14px] text-gray-200">
              <GitBranch size={14} className="text-gray-500" strokeWidth={1.5} />
              <span className="font-mono">{branch || 'unknown'}</span>
            </span>
          </div>

          {/* Changes */}
          <div className="flex items-center justify-between">
            <span className="text-[14px] text-gray-300 font-medium">Changes</span>
            {stat ? (
              <span className="flex items-center gap-2 text-[14px]">
                <span className="text-gray-300">
                  {stat.filesChanged} file{stat.filesChanged !== 1 ? 's' : ''}
                </span>
                <span className="font-mono text-green-400">+{stat.insertions}</span>
                <span className="font-mono text-red-400">-{stat.deletions}</span>
              </span>
            ) : (
              <span className="text-gray-500 text-[14px]">No changes</span>
            )}
          </div>

          {/* Include unstaged toggle */}
          <div className="flex items-center gap-3">
            <button
              onClick={() => setIncludeUnstaged(!includeUnstaged)}
              className={`relative w-10 h-[22px] rounded-full transition-colors ${
                includeUnstaged ? 'bg-blue-500' : 'bg-white/[0.12]'
              }`}
            >
              <div
                className={`absolute top-[3px] w-4 h-4 rounded-full bg-white transition-transform ${
                  includeUnstaged ? 'left-[22px]' : 'left-[3px]'
                }`}
              />
            </button>
            <span className="text-[14px] text-gray-300">Include unstaged</span>
          </div>

          {/* Commit message */}
          <div>
            <span className="text-[14px] text-gray-300 font-medium block mb-2">Commit message</span>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="Enter a commit message..."
              rows={3}
              className="w-full px-3 py-2 text-[14px] text-gray-200 bg-white/[0.04] border border-white/[0.08]
                         rounded-lg resize-none outline-none focus:border-white/[0.16] placeholder:text-gray-600
                         transition-colors"
            />
          </div>

          {/* Next steps */}
          <div>
            <span className="text-[14px] text-gray-300 font-medium block mb-2">Next steps</span>
            <div className="space-y-1">
              {actions.map((a) => {
                const Icon = a.icon
                const isSelected = action === a.id
                return (
                  <button
                    key={a.id}
                    onClick={() => setAction(a.id)}
                    className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-[14px] transition-colors
                               ${isSelected ? 'bg-white/[0.08] text-gray-100' : 'hover:bg-white/[0.04] text-gray-400'}`}
                  >
                    <Icon size={16} strokeWidth={1.5} />
                    <span className="flex-1 text-left">{a.label}</span>
                    {isSelected && <Check size={16} strokeWidth={2} className="text-gray-300" />}
                  </button>
                )
              })}
            </div>
          </div>

          {/* Error */}
          {error && (
            <div className="text-[13px] text-red-400 bg-red-500/10 rounded-lg px-3 py-2 break-words">
              {error}
            </div>
          )}

          {/* Continue button */}
          <button
            onClick={handleContinue}
            disabled={!canCommit}
            className={`w-full py-2.5 rounded-lg text-[14px] font-medium transition-colors flex items-center justify-center gap-2
                       ${
                         canCommit
                           ? 'bg-white text-gray-900 hover:bg-gray-200'
                           : 'bg-white/[0.06] text-gray-500 cursor-not-allowed'
                       }`}
          >
            {loading ? <Loader2 size={16} className="animate-spin" /> : 'Continue'}
          </button>
        </div>
      </motion.div>
    </>
  )
}
