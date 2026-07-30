import { TerminalSlot } from './TerminalSlot'
import { CommandSpine } from './CommandSpine'
import { BlockLog } from './BlockLog'
import { RunningCommand } from './RunningCommand'
import { registerBlockLogView } from '../lib/block-log'
import { useLiveTerminalRows } from '../hooks/useLiveTerminalRows'
import { hasShellIntegration, onCommandBlocksChange } from '../lib/command-blocks'
import { useCallback, useEffect, useSyncExternalStore } from 'react'
import { isShellSession } from '../../shared/types'
import type { AgentType } from '../../shared/types'

/**
 * The terminal surface: the command spine and the terminal itself, composed
 * as one unit.
 *
 * Call sites should not need to know the spine exists, let alone its width.
 * That arithmetic lives in spine-layout and terminal-indent, so changing the
 * gutter does not mean hunting for matching Tailwind literals in three
 * components and a pixel prop in a fourth.
 */

interface Props {
  terminalId: string
  agentType: AgentType | undefined
  isFocused: boolean
  /**
   * Finished commands are drawn above as elements, and the terminal below
   * holds only the live one, sized to what that command is currently drawing.
   * The pty is resized to match, so a full-screen program is given the whole
   * pane instead — see useLiveTerminalRows.
   */
  domBlocks?: boolean
  /**
   * Grid mode positions both children absolutely so the 16px south-east
   * corner stays free for the react-grid-layout resize handle, which a flex
   * row would cover.
   */
  flexible?: boolean
}

/** Row height the live region is measured in. */
const LIVE_ROW_PX = 19

export function TerminalPane({ terminalId, agentType, isFocused, flexible, domBlocks }: Props) {
  const hasSpine = isShellSession(agentType)
  // Blocks replace the terminal's own layout, so they wait for proof that the
  // shell reports command boundaries. Without it — bash, fish, PowerShell, cmd
  // — this stays false and the terminal is left exactly as it was.
  const integrated = useSyncExternalStore(
    useCallback((cb: () => void) => onCommandBlocksChange(terminalId, cb), [terminalId]),
    useCallback(() => hasShellIntegration(terminalId), [terminalId])
  )

  const splitLog = Boolean(domBlocks) && isShellSession(agentType) && integrated

  const liveRows = useLiveTerminalRows(terminalId, splitLog)
  // A full-screen program owns the pane; the log has nothing to add while it
  // is up, and its markers point at the normal buffer anyway.
  const fullScreen = liveRows === null
  const logClass = fullScreen ? 'hidden' : 'min-h-0'
  const liveClass = fullScreen ? 'flex-1 min-h-0 w-full' : 'shrink-0 w-full'
  const liveStyle = fullScreen ? undefined : { height: liveRows * LIVE_ROW_PX }

  // Declares that this terminal's finished commands are being drawn here, so
  // the capture path knows it is safe to take them out of the buffer.
  useEffect(() => {
    if (!splitLog) return
    return registerBlockLogView(terminalId)
  }, [splitLog, terminalId])

  if (flexible && splitLog) {
    // The 16px south-east reservation moves to the wrapper, so the resize
    // handle stays reachable while the log and terminal stack inside it.
    return (
      <div className="absolute inset-0 right-4 bottom-4 flex flex-col justify-end">
        <BlockLog terminalId={terminalId} className={logClass} />
        {!fullScreen && <RunningCommand terminalId={terminalId} />}
        <TerminalSlot
          terminalId={terminalId}
          isFocused={isFocused}
          className={liveClass}
          style={liveStyle}
        />
      </div>
    )
  }

  if (flexible) {
    return (
      <>
        {hasSpine && (
          <CommandSpine terminalId={terminalId} className="absolute left-0 top-0 bottom-4" />
        )}
        <TerminalSlot
          terminalId={terminalId}
          isFocused={isFocused}
          className={
            hasSpine
              ? 'absolute inset-0 left-4 right-6 bottom-4'
              : 'absolute inset-0 right-4 bottom-4'
          }
        />
      </>
    )
  }

  // Without a spine the markup is left exactly as it was before the gutter
  // existed, so agent sessions keep their layout rather than an equivalent.
  if (!hasSpine) {
    return <TerminalSlot terminalId={terminalId} isFocused={isFocused} className="w-full h-full" />
  }

  if (splitLog) {
    return (
      // justify-end keeps the live terminal at the bottom before any command
      // has run, rather than stranded at the top above empty space.
      <div className="flex h-full w-full flex-col justify-end">
        <BlockLog terminalId={terminalId} className={logClass} />
        {/* Blocks only exist once a command has finished, so a running one
            needs saying out loud — otherwise a command that never exits looks
            like a terminal that stopped responding. */}
        {!fullScreen && <RunningCommand terminalId={terminalId} />}
        {/* The live command only. Capped so a quiet session is mostly log,
            but tall enough that a running command has room to draw. */}
        <TerminalSlot
          terminalId={terminalId}
          isFocused={isFocused}
          className={liveClass}
          style={liveStyle}
        />
      </div>
    )
  }

  return (
    <div className="flex h-full w-full pr-2">
      <CommandSpine terminalId={terminalId} className="relative mr-2 h-full" />
      <TerminalSlot
        terminalId={terminalId}
        isFocused={isFocused}
        className="flex-1 min-w-0 h-full"
      />
    </div>
  )
}
