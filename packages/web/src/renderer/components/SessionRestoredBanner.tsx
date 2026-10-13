import { useState } from 'react'
import { useAppStore } from '../stores'
import { RotateCcw, X } from 'lucide-react'
import { showEndedSession } from '../lib/session-resume'

export function SessionRestoredBanner() {
  const previousSessions = useAppStore((s) => s.previousSessions)
  const setSessionBanner = useAppStore((s) => s.setSessionBanner)
  const [restoring, setRestoring] = useState(false)

  // Dismissing hides the offer for this launch and leaves the records alone.
  // It used to clear them, so a glance at the banner and a wrong click threw
  // away every terminal's history -- and there was nothing left to change your
  // mind from. The server retires them on its own once they age out.
  const handleDismiss = (): void => {
    setSessionBanner(false)
  }

  // Shows the panes. It does not relaunch anything, which is the same rule the
  // rest of start-up now follows: the view comes back, the agent does not.
  //
  // This used to build a launch payload and call `createTerminal` per session --
  // a second implementation of resume that grew apart from the real one, and one
  // that bypassed the server's claim, so two clients pressing it started two
  // agents against one transcript.
  const handleRestore = async (): Promise<void> => {
    setRestoring(true)
    try {
      for (const prev of previousSessions) await showEndedSession(prev.id)
    } finally {
      setSessionBanner(false)
      setRestoring(false)
    }
  }

  return (
    <div
      className="mx-4 mt-4 px-4 py-3 border border-white/[0.08] bg-white/[0.03]
                    rounded-lg flex items-center justify-between"
    >
      <div className="flex items-center gap-3">
        <RotateCcw size={16} className="text-gray-400 shrink-0" />
        <p className="text-sm text-gray-300">
          {previousSessions.length} previous session{previousSessions.length !== 1 ? 's' : ''} can
          be restored
          <span className="text-gray-500">
            {' · '}
            {new Set(previousSessions.map((s) => s.projectName)).size === 1
              ? previousSessions[0].projectName
              : `${new Set(previousSessions.map((s) => s.projectName)).size} projects`}
          </span>
        </p>
      </div>
      <div className="flex items-center gap-2 ml-4 shrink-0">
        <button
          onClick={handleRestore}
          disabled={restoring}
          className="px-3 py-1 text-xs font-medium text-black rounded-md transition-colors bg-bronzo hover:bg-bronzo-dark"
        >
          {restoring ? 'Restoring...' : 'Restore'}
        </button>
        <button
          onClick={handleDismiss}
          className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
          title="Dismiss"
        >
          <X size={14} />
        </button>
      </div>
    </div>
  )
}
