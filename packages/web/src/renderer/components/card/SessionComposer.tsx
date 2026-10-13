import { useAppStore } from '../../stores'
import { IntentBar } from '../IntentBar'
import { EndedStrip } from './EndedStrip'

interface Props {
  terminalId: string
  compact?: boolean
  indentPx?: number
  /**
   * Suppress the composer without suppressing the strip.
   *
   * The focused pane hides its intent bar on mobile, where the keyboard owns
   * that space. A pane whose session has ended still has something to say
   * there, and it is not a place to type.
   */
  hideIntentBar?: boolean
}

/**
 * The row beneath a terminal: a place to type, or a note saying why there is
 * not one.
 *
 * One component rather than a condition at each of the four surfaces that render
 * this row, so a pane cannot end up offering a composer for a session that
 * cannot take input -- which is what would happen the first time somebody added
 * a fifth surface and copied three of the four call sites.
 */
export function SessionComposer({ terminalId, compact, indentPx, hideIntentBar }: Props) {
  const ended = useAppStore((s) => s.terminals.get(terminalId)?.ended)
  if (ended) return <EndedStrip terminalId={terminalId} ended={ended} compact={compact} />
  if (hideIntentBar) return null
  return <IntentBar terminalId={terminalId} compact={compact} indentPx={indentPx} />
}
