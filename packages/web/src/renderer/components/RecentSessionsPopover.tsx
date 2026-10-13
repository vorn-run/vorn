import { useState, useEffect } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { getProjectRemoteHostId, type RecentSession } from '../../shared/types'
import { AgentIcon } from './AgentIcon'
import { formatRecentSessionActivity, resolveProjectName } from '../lib/session-utils'

function timeAgo(ts: number): string {
  const sec = Math.floor((Date.now() - ts) / 1000)
  if (sec < 60) return 'just now'
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`
  if (sec < 604800) return `${Math.floor(sec / 86400)}d ago`
  return new Date(ts).toLocaleDateString()
}

interface Props {
  isOpen: boolean
  onClose: () => void
}

export function RecentSessionsPopover({ isOpen, onClose }: Props) {
  const config = useAppStore((s) => s.config)
  const activeProject = useAppStore((s) => s.activeProject)
  const addTerminal = useAppStore((s) => s.addTerminal)

  const [sessions, setSessions] = useState<RecentSession[]>([])

  useEffect(() => {
    if (!isOpen) return

    let projectPath: string | undefined
    if (activeProject && config) {
      const project = config.projects.find((p) => p.name === activeProject)
      if (project) projectPath = project.path
    }

    window.api
      .getRecentSessions(projectPath)
      .then(setSessions)
      .catch(() => setSessions([]))
  }, [isOpen, activeProject, config])

  const handleResume = async (session: RecentSession): Promise<void> => {
    if (!session.canResumeExact) return

    try {
      const projectName = resolveProjectName(session, config?.projects)
      const proj = config?.projects.find((p) => p.name === projectName)
      const remoteHostId = proj ? getProjectRemoteHostId(proj) : undefined
      const result = await window.api.createTerminal({
        agentType: session.agentType,
        projectName,
        projectPath: session.projectPath,
        resumeSessionId: session.sessionId,
        remoteHostId
      })
      addTerminal(result)
      onClose()
    } catch (err) {
      console.error('[RecentSessionsPopover] failed to resume session:', err)
    }
  }

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          {/* Backdrop — titlebar-no-drag so clicks on Electron drag regions still close */}
          <div className="fixed inset-0 z-50 titlebar-no-drag" onClick={onClose} />
          <motion.div
            className="absolute top-full right-0 mt-2 z-50 w-[380px] max-h-[400px]
                       border border-white/[0.08] rounded-xl shadow-2xl overflow-hidden flex flex-col"
            style={{ background: 'var(--color-surface-overlay)' }}
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 500, damping: 30 }}
          >
            {/* Header */}
            <div className="flex items-center gap-2 px-4 py-3 border-b border-white/[0.06] shrink-0">
              <span className="text-[13px] font-medium text-white">Recent Sessions</span>
              <span className="text-[11px] text-gray-500 truncate">
                {activeProject ? `· ${activeProject}` : '· All Projects'}
              </span>
              {sessions.length > 0 && (
                <span className="text-gray-600 text-xs ml-auto shrink-0">{sessions.length}</span>
              )}
            </div>

            {/* Session list */}
            <div className="flex-1 min-h-0 overflow-auto">
              {sessions.length === 0 ? (
                <div className="flex items-center justify-center p-8">
                  <p className="text-xs text-gray-600">No recent sessions</p>
                </div>
              ) : (
                <div className="p-1.5 space-y-0.5">
                  {sessions.map((session) => {
                    const projectName = resolveProjectName(session, config?.projects)
                    return (
                      <button
                        key={session.sessionId}
                        onClick={() => handleResume(session)}
                        disabled={!session.canResumeExact}
                        className={`w-full text-left px-3 py-2 rounded-lg transition-colors group flex items-start gap-2.5 ${
                          session.canResumeExact
                            ? 'hover:bg-white/[0.06]'
                            : 'cursor-default opacity-75'
                        }`}
                      >
                        <div className="shrink-0 mt-0.5">
                          <AgentIcon agentType={session.agentType} size={14} />
                        </div>
                        <div className="flex-1 min-w-0">
                          <div className="text-[12px] text-gray-300 truncate leading-tight">
                            {session.display || 'Untitled session'}
                          </div>
                          <div className="flex items-center gap-1.5 mt-0.5">
                            {!activeProject && (
                              <span className="text-[10px] text-gray-600 truncate">
                                {projectName}
                              </span>
                            )}
                            {!activeProject && <span className="text-[10px] text-gray-700">·</span>}
                            <span className="text-[10px] text-gray-600 shrink-0">
                              {timeAgo(session.timestamp)}
                            </span>
                            <span className="text-[10px] text-gray-700">·</span>
                            <span className="text-[10px] text-gray-600 shrink-0">
                              {formatRecentSessionActivity(session)}
                            </span>
                          </div>
                        </div>
                        <span
                          className={`text-[10px] shrink-0 mt-0.5 ${
                            session.canResumeExact
                              ? 'text-gray-700 opacity-0 group-hover:opacity-100 transition-opacity'
                              : 'text-gray-600'
                          }`}
                        >
                          {session.canResumeExact ? 'resume' : 'history only'}
                        </span>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
