import { SPINE_GAP_PX, SPINE_WIDTH_PX } from './spine-layout'
import { isShellSession } from '../../shared/types'
import type { AgentType } from '../../shared/types'

/**
 * Left inset of the terminal's text column, for anything aligning to it.
 *
 * Lives beside the layout constants it is derived from rather than in the
 * component, so changing the gutter does not mean hunting for matching Tailwind
 * literals and a pixel prop in separate files.
 */

/** Matches a block's own left padding, so the caret shares its column. */
export const BLOCK_TEXT_INSET_PX = 12

export function terminalTextIndentPx(agentType: AgentType | undefined, domBlocks = false): number {
  if (!isShellSession(agentType)) return 0
  // With blocks drawn as elements there is no gutter — the caret lines up with
  // a block's own left padding instead.
  if (domBlocks) return BLOCK_TEXT_INSET_PX
  return SPINE_WIDTH_PX + SPINE_GAP_PX
}
