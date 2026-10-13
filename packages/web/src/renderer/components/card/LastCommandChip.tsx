import { useCallback, useEffect, useState } from 'react'
import { useAppStore } from '../../stores'
import { isShellSession } from '../../../shared/types'
import { formatDuration, getCommandBlocks, onCommandBlocksChange } from '../../lib/command-blocks'
import { onTerminalReady, scrollTerminalToLine } from '../../lib/terminal-registry'

interface Props {
  terminalId: string
}

/**
 * The most recent finished command, in the card's status bar.
 *
 * The spine says where commands are; this says what the last one did. It
 * carries the information the old inline exit chip did, in a row Vorn owns,
 * so nothing is painted over the terminal to get it.
 */
export function LastCommandChip({ terminalId }: Props) {
  const agentType = useAppStore((s) => s.terminals.get(terminalId)?.session.agentType)
  const [last, setLast] = useState<{
    command: string | null
    exitCode: number
    durationMs: number
    line: number
  } | null>(null)

  const isShell = isShellSession(agentType)

  const refresh = useCallback(() => {
    const blocks = getCommandBlocks(terminalId)
    const block = blocks[blocks.length - 1]
    if (!block || block.marker.isDisposed) {
      setLast(null)
      return
    }
    setLast({
      command: block.command,
      exitCode: block.exitCode,
      durationMs: block.durationMs,
      line: block.marker.line
    })
  }, [terminalId])

  useEffect(() => {
    if (!isShell) return
    const disposeReady = onTerminalReady(terminalId, refresh)
    const disposeBlocks = onCommandBlocksChange(terminalId, refresh)
    return () => {
      disposeReady()
      disposeBlocks()
    }
  }, [terminalId, isShell, refresh])

  if (!isShell || !last || !last.command) return null

  const ok = last.exitCode === 0

  return (
    <button
      type="button"
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation()
        scrollTerminalToLine(terminalId, last.line)
      }}
      title={`${last.command} — ${ok ? 'succeeded' : `exit ${last.exitCode}`} in ${formatDuration(last.durationMs)}`}
      className="flex items-center gap-1 min-w-0 shrink text-[10px] text-gray-500
                 hover:text-gray-300 transition-colors"
    >
      {/* Success is the ordinary case and recedes; only failure takes colour. */}
      <span className={ok ? 'text-ink-faint' : 'text-danger'}>{ok ? '✓' : '✗'}</span>
      {!ok && <span className="text-danger font-mono">{last.exitCode}</span>}
      <span className="font-mono tabular-nums">{formatDuration(last.durationMs)}</span>
      <span className="font-mono truncate max-w-[160px] text-gray-600">{last.command}</span>
    </button>
  )
}
