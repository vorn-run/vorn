import { useCallback, useEffect, useState } from 'react'
import {
  scrollToBottom,
  isAtBottom,
  onTerminalReady,
  onTerminalScroll
} from '../lib/terminal-registry'

export function useTerminalScrollButton(terminalId: string | null | undefined) {
  const [showScrollBtn, setShowScrollBtn] = useState(false)

  useEffect(() => {
    if (!terminalId) return

    let scrollDispose: (() => void) | undefined
    const check = (): void => setShowScrollBtn(!isAtBottom(terminalId))

    const readyDispose = onTerminalReady(terminalId, () => {
      check()
      scrollDispose = onTerminalScroll(terminalId, check)
    })

    return () => {
      readyDispose()
      scrollDispose?.()
      setShowScrollBtn(false)
    }
  }, [terminalId])

  const handleScrollToBottom = useCallback(() => {
    if (!terminalId) return
    scrollToBottom(terminalId)
    setShowScrollBtn(false)
  }, [terminalId])

  return { showScrollBtn, handleScrollToBottom }
}
