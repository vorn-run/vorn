import { TerminalSlot } from './TerminalSlot'
import { CommandSpine } from './CommandSpine'
import { SPINE_GAP_PX, SPINE_WIDTH_PX } from '../lib/spine-layout'
import { isShellSession } from '../../shared/types'
import type { AgentType } from '../../shared/types'

/**
 * The terminal surface: the command spine and the terminal itself, composed
 * as one unit.
 *
 * Call sites should not need to know the spine exists, let alone its width —
 * that arithmetic lives here and in SPINE_WIDTH_PX/SPINE_GAP_PX, so changing
 * the gutter does not mean hunting for matching Tailwind literals in three
 * components and a pixel prop in a fourth.
 */

interface Props {
  terminalId: string
  agentType: AgentType | undefined
  isFocused: boolean
  /**
   * Grid mode positions both children absolutely so the 16px south-east
   * corner stays free for the react-grid-layout resize handle, which a flex
   * row would cover.
   */
  flexible?: boolean
}

/** Left inset of the terminal's text column, for anything aligning to it. */
export function terminalTextIndentPx(agentType: AgentType | undefined): number {
  return isShellSession(agentType) ? SPINE_WIDTH_PX + SPINE_GAP_PX : 0
}

export function TerminalPane({ terminalId, agentType, isFocused, flexible }: Props) {
  const hasSpine = isShellSession(agentType)

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
