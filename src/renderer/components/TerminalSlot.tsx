import { useEffect, useRef } from 'react'
import { registerSlot, unregisterSlot, focusTerminal } from '../lib/terminal-registry'
import { useOnScreenOnce } from '../hooks/useOnScreenOnce'
import { useOnScreen } from '../hooks/useOnScreen'
import { markVisible, markHidden } from '../lib/visible-terminals'

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
  // Registering is what builds the xterm and pulls the session's scrollback, up
  // to 256KB of it. Every card on the board mounts one of these, so a phone
  // opening a board of ten paid for ten before reading anything -- and again on
  // every reconnect, which on a phone is every network change. Waiting until the
  // slot is near the viewport spends that on the cards somebody is looking at.
  const onScreen = useOnScreenOnce(ref)
  // Tracked, not latched: the web client asks the server only for the bytes of
  // cards on screen, and that has to follow the scroll both ways.
  const near = useOnScreen(ref)
  useEffect(() => {
    if (!near) return
    markVisible(terminalId)
    return () => markHidden(terminalId)
  }, [terminalId, near])

  useEffect(() => {
    const el = ref.current
    if (!el || !onScreen) return
    registerSlot(terminalId, el)
    return () => {
      unregisterSlot(terminalId, el)
    }
  }, [terminalId, onScreen])

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
