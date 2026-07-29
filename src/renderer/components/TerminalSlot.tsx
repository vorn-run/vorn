import { useEffect, useRef } from 'react'
import { registerSlot, unregisterSlot, focusTerminal } from '../lib/terminal-registry'
import { useStatusDetection } from '../hooks/useStatusDetection'

interface Props {
  terminalId: string
  isFocused: boolean
  className?: string
  /** The overlay tracks this element's rect, so sizing it here sizes the pty. */
  style?: React.CSSProperties
}

/**
 * A placeholder element that declares "this view wants the terminal rendered
 * here." The actual xterm DOM lives permanently in the singleton TerminalHost
 * and is positioned to overlay this element via fixed-position CSS. Unmounting
 * this component hides the terminal; it does not destroy or reparent it.
 */
export function TerminalSlot({ terminalId, isFocused, className, style }: Props) {
  const ref = useRef<HTMLDivElement>(null)

  useStatusDetection(terminalId)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    registerSlot(terminalId, el)
    return () => {
      unregisterSlot(terminalId, el)
    }
  }, [terminalId])

  useEffect(() => {
    if (!isFocused) return
    // Defer focus by one frame so the slot has been positioned and the
    // browser has applied the overlay's visibility:visible before we try
    // to move the keyboard focus onto it.
    const rafId = requestAnimationFrame(() => focusTerminal(terminalId))
    return () => cancelAnimationFrame(rafId)
  }, [isFocused, terminalId])

  return <div ref={ref} className={className} style={style} />
}
