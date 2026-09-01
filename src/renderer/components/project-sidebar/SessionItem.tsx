import { useRef, useCallback, useEffect, useState } from 'react'
import { X, FolderTree, Globe, Loader2, Smartphone, SquareTerminal } from 'lucide-react'
import { useAppStore } from '../../stores'
import { AgentStatusIcon } from '../AgentStatusIcon'
import { closeTerminalSession } from '../../lib/terminal-close'
import { toggleTerminalsPanel } from '../../lib/session-utils'
import { toast } from '../Toast'
import { STATUS_LABEL } from '../../lib/status-colors'
import { DevicePicker } from '../DevicePicker'
import { shouldShowDeviceButton } from '../../lib/device-affordance'
import { shouldOfferPane } from '../../lib/pane-affordance'
import { PromotedCardItem } from './PromotedCardItem'
import { usePromotedCardsFor } from '../../hooks/usePromotedCards'
import type { SidebarSessionInfo } from './types'

const PREVIEW_DELAY_MS = 300

export function SessionItem({
  session,
  showBranch = true
}: {
  session: SidebarSessionInfo
  showBranch?: boolean
}) {
  const focusedTerminalId = useAppStore((s) => s.focusedTerminalId)
  const activeTabId = useAppStore((s) => s.activeTabId)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const setActiveTabId = useAppStore((s) => s.setActiveTabId)
  const setPreviewTerminal = useAppStore((s) => s.setPreviewTerminal)
  const previewTerminalId = useAppStore((s) => s.previewTerminalId)
  const layoutMode = useAppStore((s) => s.config?.defaults?.layoutMode ?? 'grid')
  const enableHoverPreview = useAppStore((s) => s.config?.defaults?.enableHoverPreview ?? false)
  const hasFilesPane = useAppStore((s) => s.filesPanes.has(session.id))
  const toggleFilesPane = useAppStore((s) => s.toggleFilesPane)
  const hasBrowserPane = useAppStore((s) => s.browserPanes.has(session.id))
  const hasTerminalsPane = useAppStore((s) => s.terminalsPanes.has(session.id))
  const toggleBrowserPane = useAppStore((s) => s.toggleBrowserPane)
  const hasDevicePane = useAppStore((s) => s.devicePanes.has(session.id))
  const claimAndOpenDevicePane = useAppStore((s) => s.claimAndOpenDevicePane)
  const closeDevicePane = useAppStore((s) => s.closeDevicePane)
  const projectPath = useAppStore((s) => s.terminals.get(session.id)?.session.projectPath ?? '')
  const mobile = useAppStore((s) => s.mobileProjectCache.get(projectPath))
  const loadMobileProject = useAppStore((s) => s.loadMobileProject)
  const promotedCards = usePromotedCardsFor(session.id)
  const [isPickerOpen, setIsPickerOpen] = useState(false)
  // A claim in flight. Boot plus `bootstatus -b` can run tens of seconds, and
  // until this existed nothing on screen said so.
  const [claiming, setClaiming] = useState(false)
  const deviceButtonRef = useRef<HTMLButtonElement>(null)

  // Probing is a readdir behind an IPC hop, so it runs off the render path and
  // the button simply appears when the answer arrives. The store dedupes by
  // project path, so every session row for one project costs a single probe.
  useEffect(() => {
    if (projectPath) void loadMobileProject(projectPath)
  }, [projectPath, loadMobileProject])

  // A shell has no use for a browser, a simulator, or a panel of further
  // shells. An already-open pane keeps its control so it can still be closed.
  const showBrowser = shouldOfferPane(session.agentType, hasBrowserPane)
  const showTerminals = shouldOfferPane(session.agentType, hasTerminalsPane)
  const showDevice = shouldShowDeviceButton(mobile, hasDevicePane, session.agentType)
  const isActive =
    layoutMode === 'tabs' ? activeTabId === session.id : focusedTerminalId === session.id
  const isPreviewing = previewTerminalId === session.id
  const isSelected = isActive || isPreviewing

  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current)
    }
  }, [])

  const handleMouseEnter = useCallback(() => {
    if (!enableHoverPreview) return
    if (layoutMode === 'tabs') return
    if (focusedTerminalId === session.id) return
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current)
    hoverTimerRef.current = setTimeout(() => {
      hoverTimerRef.current = null
      setPreviewTerminal(session.id)
    }, PREVIEW_DELAY_MS)
  }, [enableHoverPreview, layoutMode, focusedTerminalId, session.id, setPreviewTerminal])

  const handleMouseLeave = useCallback(() => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current)
      hoverTimerRef.current = null
    }
    if (previewTerminalId === session.id) {
      setPreviewTerminal(null)
    }
  }, [previewTerminalId, session.id, setPreviewTerminal])

  return (
    // A fragment, because the cards this session popped out are listed beneath
    // it — and they cannot go inside the row, which is itself a button.
    <>
      {/* A div with the button role: the row nests real buttons inside it. */}
      <div
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            e.currentTarget.click()
          }
        }}
        onClick={() => {
          if (hoverTimerRef.current) {
            clearTimeout(hoverTimerRef.current)
            hoverTimerRef.current = null
          }
          if (layoutMode === 'tabs') {
            setActiveTabId(session.id)
            setFocusedTerminal(null)
          } else {
            setFocusedTerminal(session.id)
          }
        }}
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        className={`group/session relative w-full cursor-pointer text-left px-2 py-1 rounded-md text-[12px] flex items-center gap-2 min-w-0 transition-colors ${
          isSelected ? 'text-white' : 'text-gray-400 hover:text-white hover:bg-white/[0.04]'
        }`}
      >
        {isSelected && (
          <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />
        )}
        <span className="shrink-0" title={`${session.agentType} · ${STATUS_LABEL[session.status]}`}>
          <AgentStatusIcon agentType={session.agentType} status={session.status} size={14} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate">{session.name}</div>
          {showBranch && session.branch && (
            <div className="text-[10px] text-gray-600 truncate">{session.branch}</div>
          )}
        </div>
        <button
          type="button"
          aria-label={`${hasFilesPane ? 'Hide' : 'Show'} files for ${session.name}`}
          title={hasFilesPane ? 'Hide files' : 'Show files'}
          onClick={(e) => {
            e.stopPropagation()
            toggleFilesPane(session.id)
          }}
          className={`${
            hasFilesPane
              ? 'opacity-100 text-ink'
              : 'opacity-0 group-hover/session:opacity-100 text-gray-500'
          } focus:opacity-100 hover:text-gray-200 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0`}
        >
          <FolderTree size={12} strokeWidth={2} />
        </button>
        {showTerminals && (
          <button
            type="button"
            aria-label={`${hasTerminalsPane ? 'Close' : 'Show'} terminals for ${session.name}`}
            title={hasTerminalsPane ? 'Close these terminals' : 'Add a terminal'}
            onClick={(e) => {
              e.stopPropagation()
              void toggleTerminalsPanel(session.id)
            }}
            className={`${
              hasTerminalsPane
                ? 'opacity-100 text-ink'
                : 'opacity-0 group-hover/session:opacity-100 text-gray-500'
            } focus:opacity-100 hover:text-gray-200 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0`}
          >
            <SquareTerminal size={12} strokeWidth={2} />
          </button>
        )}
        {showBrowser && (
          <button
            type="button"
            aria-label={`${hasBrowserPane ? 'Hide' : 'Show'} browser for ${session.name}`}
            title={hasBrowserPane ? 'Hide browser' : 'Show browser'}
            onClick={(e) => {
              e.stopPropagation()
              toggleBrowserPane(session.id)
            }}
            className={`${
              hasBrowserPane
                ? 'opacity-100 text-ink'
                : 'opacity-0 group-hover/session:opacity-100 text-gray-500'
            } focus:opacity-100 hover:text-gray-200 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0`}
          >
            <Globe size={12} strokeWidth={2} />
          </button>
        )}
        {showDevice && (
          <button
            ref={deviceButtonRef}
            type="button"
            aria-label={`${hasDevicePane ? 'Hide' : 'Show'} device for ${session.name}`}
            title={
              hasDevicePane ? 'Hide device' : claiming ? 'Booting the simulator…' : 'Show device'
            }
            disabled={claiming}
            aria-busy={claiming}
            onClick={(e) => {
              e.stopPropagation()
              if (claiming) return
              if (hasDevicePane) closeDevicePane(session.id)
              else setIsPickerOpen((v) => !v)
            }}
            className={`${
              hasDevicePane
                ? 'opacity-100 text-ink'
                : 'opacity-0 group-hover/session:opacity-100 text-gray-500'
            } ${
              claiming ? 'opacity-100 cursor-wait' : ''
            } focus:opacity-100 hover:text-gray-200 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0`}
          >
            {/* Boot plus `bootstatus -b` runs tens of seconds on a cold
              simulator. Without a spinner the row looked inert and the natural
              response was to click again, which could start a second claim on a
              different device. */}
            {claiming ? (
              <Loader2 size={12} strokeWidth={2} className="animate-spin" />
            ) : (
              <Smartphone size={12} strokeWidth={2} />
            )}
          </button>
        )}
        {isPickerOpen && (
          <DevicePicker
            sessionId={session.id}
            anchorRef={deviceButtonRef}
            onClose={() => setIsPickerOpen(false)}
            onSelect={(device) => {
              setIsPickerOpen(false)
              setClaiming(true)
              void claimAndOpenDevicePane(session.id, device).then((err) => {
                setClaiming(false)
                // The likeliest failure is another session holding the device,
                // and that message names the holder. A toast rather than inline
                // state because the picker has already closed by now.
                if (err) toast.error(err)
              })
            }}
          />
        )}
        <button
          type="button"
          aria-label={`Close session ${session.name}`}
          title="Close session"
          onClick={async (e) => {
            e.stopPropagation()
            await closeTerminalSession(session.id)
            toast.success('Session closed')
          }}
          className="opacity-0 group-hover/session:opacity-100 focus:opacity-100 text-gray-500 hover:text-red-400 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0"
        >
          <X size={12} strokeWidth={2} />
        </button>
      </div>
      {promotedCards.map((card) => (
        <PromotedCardItem key={card.id} card={card} />
      ))}
    </>
  )
}
