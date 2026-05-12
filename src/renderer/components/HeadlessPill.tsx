import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { HeadlessSession } from '../../shared/types'
import { AgentIcon } from './AgentIcon'
import { useAppStore } from '../stores'
import { ICON_MAP } from './project-sidebar/icon-map'
import { GitBranch, X, Square, Workflow } from 'lucide-react'

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rs = s % 60
  if (m < 60) return rs > 0 ? `${m}m${rs}s` : `${m}m`
  const h = Math.floor(m / 60)
  return `${h}h${m % 60}m`
}

interface Props {
  session: HeadlessSession
}

export function HeadlessPill({ session }: Props) {
  const [expanded, setExpanded] = useState(false)
  const [logs, setLogs] = useState('')
  const [hovered, setHovered] = useState(false)
  const [duration, setDuration] = useState('')
  const logsRef = useRef<HTMLDivElement>(null)
  const lastOutput = useAppStore((s) => s.headlessLastOutput.get(session.id))
  const { dismissHeadless, setEditingWorkflowId, setWorkflowEditorOpen, workflows } = useAppStore(
    useShallow((s) => ({
      dismissHeadless: s.dismissHeadlessSession,
      setEditingWorkflowId: s.setEditingWorkflowId,
      setWorkflowEditorOpen: s.setWorkflowEditorOpen,
      workflows: s.config?.workflows
    }))
  )

  const workflow = useMemo(
    () => (session.workflowId ? workflows?.find((w) => w.id === session.workflowId) : undefined),
    [session.workflowId, workflows]
  )

  // Tick duration for running sessions
  useEffect(() => {
    const compute = (): string => {
      const now = Date.now()
      const ms =
        session.status === 'running'
          ? now - session.startedAt
          : (session.endedAt ?? now) - session.startedAt
      return formatDuration(ms)
    }
    if (session.status !== 'running') {
      // For exited sessions, compute once via interval that fires immediately then clears
      const timer = setTimeout(() => setDuration(compute()), 0)
      return () => clearTimeout(timer)
    }
    const interval = setInterval(() => setDuration(compute()), 1000)
    return () => clearInterval(interval)
  }, [session.status, session.startedAt, session.endedAt])

  // Subscribe to headless data only when expanded
  useEffect(() => {
    if (!expanded) return
    const remove = window.api.onHeadlessData(({ id, data }: { id: string; data: string }) => {
      if (id === session.id) {
        setLogs((prev) => {
          const next = prev + data
          // Cap at 100KB
          return next.length > 100_000 ? next.slice(-80_000) : next
        })
      }
    })
    return remove
  }, [expanded, session.id])

  // Auto-scroll logs
  useEffect(() => {
    if (logsRef.current) {
      logsRef.current.scrollTop = logsRef.current.scrollHeight
    }
  }, [logs])

  const handleDismiss = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      dismissHeadless(session.id)
    },
    [dismissHeadless, session.id]
  )

  const handleKill = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      window.api.killHeadlessSession(session.id)
    },
    [session.id]
  )

  const isRunning = session.status === 'running'
  const isError = session.status === 'exited' && session.exitCode !== 0

  const borderClass = isRunning
    ? 'animate-[borderPulse_2.5s_ease-in-out_infinite]'
    : isError
      ? 'border-red-500/25'
      : 'border-white/[0.06]'

  const opacityClass = !isRunning ? 'opacity-[0.65]' : ''

  const WfIcon = workflow ? ICON_MAP[workflow.icon] || Workflow : null
  const wfIconColor = workflow?.iconColor
  const showWorkflowTag = !!(WfIcon && session.workflowName)

  return (
    <div
      className={`inline-flex rounded-md border bg-[#1a1a1e] px-2.5 py-1
                   cursor-pointer transition-[border-color,box-shadow,opacity] select-none
                   hover:border-white/[0.12]
                   ${borderClass} ${opacityClass}
                   ${showWorkflowTag || expanded ? 'flex-col !items-stretch' : 'items-center gap-1.5'}
                   ${expanded ? 'w-[320px]' : ''}`}
      onClick={() => setExpanded(!expanded)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {/* Main pill row */}
      <div className="flex items-center gap-1.5 min-w-0">
        {/* Status dot */}
        <div className="relative flex-shrink-0">
          <div
            className={`w-1.5 h-1.5 rounded-full ${
              isRunning ? 'bg-green-500' : isError ? 'bg-red-500' : 'bg-gray-500'
            }`}
          />
          {isRunning && (
            <div
              className="absolute inset-[-1px] rounded-full bg-green-500 opacity-40 animate-ping"
              style={{ animationDuration: '2s' }}
            />
          )}
        </div>

        <AgentIcon agentType={session.agentType} size={14} />

        <span className="text-[11px] font-medium text-gray-200 truncate max-w-[100px]">
          {session.displayName || session.projectName}
        </span>

        <span className="text-[10px] text-gray-600 flex-shrink-0">&middot;</span>

        {session.branch && (
          <span className="flex items-center gap-0.5 text-[10px] font-mono text-gray-500 truncate max-w-[90px]">
            <GitBranch size={9} strokeWidth={1.5} className="flex-shrink-0" />
            {session.branch}
          </span>
        )}

        {/* Last output snippet (only when collapsed and not hovered) */}
        {!expanded && !hovered && lastOutput && (
          <>
            <span className="text-[10px] text-gray-600 flex-shrink-0">&middot;</span>
            <span className="text-[10px] font-mono text-gray-600 truncate max-w-[140px]">
              {lastOutput}
            </span>
          </>
        )}

        {/* Duration (hidden when hovered) */}
        {!hovered && (
          <span className="text-[10px] font-mono text-gray-500 flex-shrink-0 ml-auto">
            {duration}
          </span>
        )}

        {/* Exit code badge */}
        {session.exitCode != null && !hovered && (
          <span
            className={`text-[9px] font-mono px-1 py-px rounded flex-shrink-0 ${
              session.exitCode === 0
                ? 'bg-green-500/10 text-green-400'
                : 'bg-red-500/10 text-red-400'
            }`}
          >
            {session.exitCode === 0 ? '\u2713' : '\u2717'} {session.exitCode}
          </span>
        )}

        {/* Action buttons on hover */}
        {hovered && (
          <div className="flex items-center gap-1 ml-auto flex-shrink-0">
            {isRunning && (
              <button
                onClick={handleKill}
                className="p-0.5 rounded text-red-400 hover:text-red-300 hover:bg-red-500/10 transition-colors"
                title="Kill process"
              >
                <Square size={10} strokeWidth={2} />
              </button>
            )}
            <button
              onClick={handleDismiss}
              className="p-0.5 rounded text-gray-500 hover:text-gray-300 hover:bg-white/[0.06] transition-colors"
              title="Hide"
            >
              <X size={10} strokeWidth={2} />
            </button>
          </div>
        )}
      </div>

      {showWorkflowTag && (
        <button
          className="flex items-center gap-1 -mt-0.5 ml-5 hover:text-gray-300 transition-colors"
          onClick={(e) => {
            e.stopPropagation()
            if (session.workflowId) {
              setEditingWorkflowId(session.workflowId)
              setWorkflowEditorOpen(true)
            }
          }}
        >
          <WfIcon size={9} strokeWidth={1.5} color={wfIconColor || undefined} />
          <span className="text-[10px] text-gray-500 truncate max-w-[120px] hover:text-gray-300">
            {session.workflowName}
          </span>
        </button>
      )}

      {/* Expanded: log output */}
      {expanded && (
        <div
          ref={logsRef}
          className="mt-1.5 bg-[#141416] rounded border border-white/[0.06] max-h-[180px] overflow-y-auto p-2"
        >
          <pre className="text-[10px] font-mono text-gray-400 whitespace-pre-wrap break-all leading-relaxed m-0">
            {logs || lastOutput || 'Waiting for output...'}
          </pre>
        </div>
      )}
    </div>
  )
}
