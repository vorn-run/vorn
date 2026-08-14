import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useAppStore } from '../stores'
import { isShellSession } from '../../shared/types'
import {
  formatDuration,
  getCommandBlocks,
  getRunningBlock,
  onCommandBlocksChange
} from '../lib/command-blocks'
import {
  computeSpineMarks,
  SPINE_WIDTH_PX,
  type SpineMark,
  type SpineStatus
} from '../lib/spine-layout'
import {
  getTerminalBufferMetrics,
  highlightTerminalBlock,
  onTerminalReady,
  onTerminalRowHover,
  onTerminalScroll,
  scrollTerminalToLine
} from '../lib/terminal-registry'

interface Props {
  terminalId: string
  /**
   * Must establish a positioning context (`relative` or `absolute`) — every
   * mark is positioned against this element. The class is the caller's because
   * Tailwind emits `.relative` after `.absolute`, so a hardcoded `relative`
   * here would silently beat an `absolute` passed in and collapse the gutter
   * to zero height.
   */
  className: string
}

/**
 * The command spine: a gutter beside the terminal, one mark per command,
 * aligned to the rows that command occupies. Hovering a mark lights up its
 * block; clicking scrolls to it.
 *
 * Shell sessions only — an agent session paints its own full-screen
 * interface in the alternate buffer, where there are no command boundaries
 * to mark.
 */

const MARK_COLOR: Record<SpineStatus, string> = {
  // Success is muted on purpose: colour is spent only where there is
  // something to report. Light enough to read as a bar against the card ground.
  ok: '#6b6b76',
  fail: '#f87171',
  running: '#60a5fa'
}

export function CommandSpine({ terminalId, className }: Props) {
  const agentType = useAppStore((s) => s.terminals.get(terminalId)?.session.agentType)
  const rootRef = useRef<HTMLDivElement>(null)
  const rafRef = useRef<number | null>(null)
  const heightRef = useRef(0)
  const [marks, setMarks] = useState<SpineMark[]>([])
  const [hoveredKey, setHoveredKey] = useState<string | null>(null)
  // Refs so the hover handler stays stable while marks change underneath it.
  const marksRef = useRef<SpineMark[]>([])
  const hoveredKeyRef = useRef<string | null>(null)
  const [hovered, setHovered] = useState<{ mark: SpineMark; top: number; left: number } | null>(
    null
  )

  const isShell = isShellSession(agentType)

  const recompute = useCallback(() => {
    const metrics = getTerminalBufferMetrics(terminalId)
    const height = heightRef.current
    if (!metrics || height <= 0) {
      setMarks([])
      return
    }
    const running = getRunningBlock(terminalId)
    const next = computeSpineMarks(
      getCommandBlocks(terminalId),
      metrics,
      height,
      running
        ? { command: running.command, since: running.since, line: running.marker.line }
        : null,
      Date.now()
    )
    marksRef.current = next
    setMarks(next)
  }, [terminalId])

  // Scroll fires on every wheel tick and there can be a dozen shell cards on
  // screen, so collapse bursts into one recompute per frame.
  const scheduleRecompute = useCallback(() => {
    if (rafRef.current !== null) return
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null
      recompute()
    })
  }, [recompute])

  // Track our own height; marks are positioned in pixels against it.
  useEffect(() => {
    const el = rootRef.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(() => {
      heightRef.current = el.clientHeight
      scheduleRecompute()
    })
    observer.observe(el)
    heightRef.current = el.clientHeight
    scheduleRecompute()
    return () => observer.disconnect()
  }, [isShell, scheduleRecompute])

  // Hovering the output itself is what people reach for; the gutter is only
  // 8px wide and easy to miss entirely.
  const handleRowHover = useCallback(
    (line: number | null) => {
      const found =
        line === null
          ? null
          : (marksRef.current.find((m) => line >= m.line && line <= m.endLine) ?? null)
      if (found?.key === hoveredKeyRef.current) return
      hoveredKeyRef.current = found?.key ?? null
      setHoveredKey(found?.key ?? null)
      highlightTerminalBlock(
        terminalId,
        found ? { startLine: found.line, endLine: found.endLine } : null
      )
    },
    [terminalId]
  )

  useEffect(() => {
    if (!isShell) return
    let disposeScroll: (() => void) | undefined
    let disposeHover: (() => void) | undefined
    const disposeReady = onTerminalReady(terminalId, () => {
      recompute()
      disposeScroll = onTerminalScroll(terminalId, scheduleRecompute)
      disposeHover = onTerminalRowHover(terminalId, handleRowHover)
    })
    const disposeBlocks = onCommandBlocksChange(terminalId, scheduleRecompute)
    return () => {
      disposeReady()
      disposeScroll?.()
      disposeHover?.()
      disposeBlocks()
      highlightTerminalBlock(terminalId, null)
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
      rafRef.current = null
    }
  }, [terminalId, isShell, recompute, scheduleRecompute, handleRowHover])

  if (!isShell) return null

  return (
    <div
      ref={rootRef}
      style={{ width: SPINE_WIDTH_PX }}
      className={`shrink-0 ${className ?? ''}`}
      onMouseLeave={() => {
        setHovered(null)
        highlightTerminalBlock(terminalId, null)
      }}
    >
      {/* Track. Faint enough that the block marks sit above it, not in it. */}
      <div className="absolute left-1/2 -translate-x-1/2 w-px inset-y-0 bg-white/[0.035]" />

      {marks.map((mark) => (
        <button
          key={mark.key}
          type="button"
          aria-label={`${mark.command ?? 'command'} · exit ${mark.exitCode}`}
          onClick={() => scrollTerminalToLine(terminalId, mark.line)}
          onMouseEnter={(e) => {
            const rect = e.currentTarget.getBoundingClientRect()
            setHovered({ mark, top: rect.top + rect.height / 2, left: rect.right + 8 })
            // Light up the rows this mark stands for, so the gutter and the
            // output are visibly the same thing.
            highlightTerminalBlock(terminalId, {
              startLine: mark.line,
              endLine: mark.endLine
            })
          }}
          className={`absolute left-1/2 -translate-x-1/2 rounded-[1px]
                      transition-all hover:opacity-100
                      ${mark.status === 'running' ? 'motion-safe:animate-pulse' : ''}`}
          style={{
            top: `${mark.y}px`,
            height: `${mark.height}px`,
            // A thin rule, not a slab. Routine commands recede further, so
            // the distinction is weight rather than another colour.
            // The hovered block's mark widens and brightens, so the gutter
            // confirms which block the highlight belongs to.
            width: hoveredKey === mark.key ? 3 : mark.routine ? 1 : 2,
            opacity:
              hoveredKey === mark.key
                ? 1
                : mark.status === 'ok'
                  ? mark.routine
                    ? 0.3
                    : 0.55
                  : 0.85,
            background: MARK_COLOR[mark.status]
          }}
        />
      ))}

      {/* The tooltip extends over the terminal, which renders in a fixed
          overlay layer (z-45). An in-card element would paint underneath it,
          so this portals to the body like the intent bar's menu does. */}
      {hovered &&
        createPortal(
          <div
            className="fixed z-[100] px-2 py-1 rounded-md text-[11px] text-gray-200 whitespace-nowrap
                       border border-white/[0.08] pointer-events-none flex items-center gap-2"
            style={{
              background: 'var(--color-surface-overlay)',
              top: hovered.top,
              left: hovered.left,
              transform: 'translate(0, -50%)'
            }}
          >
            <span className="font-mono truncate max-w-[280px]">
              {hovered.mark.command ?? 'command'}
            </span>
            {hovered.mark.count > 1 && (
              <span className="text-gray-500 font-mono text-[10px]">+{hovered.mark.count - 1}</span>
            )}
            <span className="text-gray-500 font-mono text-[10px]">
              {hovered.mark.status === 'running'
                ? 'running'
                : hovered.mark.exitCode === 0
                  ? formatDuration(hovered.mark.durationMs)
                  : `exit ${hovered.mark.exitCode} · ${formatDuration(hovered.mark.durationMs)}`}
            </span>
          </div>,
          document.body
        )}
    </div>
  )
}
