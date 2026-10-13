import { useEffect, useRef, useState } from 'react'
import { setHostRoot, startTerminalOverlaySync, TERMINAL_ID_ATTR } from '../lib/terminal-registry'
import { useAppStore } from '../stores'
import { TerminalContextMenu } from './TerminalContextMenu'

interface CtxMenuState {
  terminalId: string
  x: number
  y: number
}

/**
 * The root every terminal wrapper is drawn in. The wrappers follow their slots
 * through `startTerminalOverlaySync`, which only reads layout while something
 * may be moving: see `overlay-sync.ts`.
 */
export function TerminalHost() {
  const rootRef = useRef<HTMLDivElement>(null)
  const [ctxMenu, setCtxMenu] = useState<CtxMenuState | null>(null)

  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    setHostRoot(el)

    const handleContextMenu = (e: MouseEvent): void => {
      const target = e.target as HTMLElement | null
      const wrapper = target?.closest(`[${TERMINAL_ID_ATTR}]`) as HTMLElement | null
      if (!wrapper) return
      const terminalId = wrapper.getAttribute(TERMINAL_ID_ATTR)
      if (!terminalId) return
      e.preventDefault()
      setCtxMenu({ terminalId, x: e.clientX, y: e.clientY })
    }
    el.addEventListener('contextmenu', handleContextMenu)

    const handlePointerDown = (e: PointerEvent): void => {
      const target = e.target as HTMLElement | null
      const wrapper = target?.closest(`[${TERMINAL_ID_ATTR}]`) as HTMLElement | null
      if (!wrapper) return
      const terminalId = wrapper.getAttribute(TERMINAL_ID_ATTR)
      if (!terminalId) return
      const state = useAppStore.getState()
      if (state.selectedTerminalId !== terminalId && state.focusedTerminalId !== terminalId) {
        state.setSelectedTerminal(terminalId)
      }
    }
    el.addEventListener('pointerdown', handlePointerDown)

    const stopSync = startTerminalOverlaySync(el)

    return () => {
      el.removeEventListener('contextmenu', handleContextMenu)
      el.removeEventListener('pointerdown', handlePointerDown)
      stopSync()
      setHostRoot(null)
    }
  }, [])

  return (
    <>
      {/* Sits above FocusedTerminal's mobile panel (z-40) and below popovers
          (z-50+) so every popover renders on top of the terminal overlay.
          Pointer events disabled on the root — wrappers opt back in when active. */}
      <div ref={rootRef} className="fixed inset-0 pointer-events-none z-[45]" />
      {ctxMenu && (
        <TerminalContextMenu
          terminalId={ctxMenu.terminalId}
          position={{ x: ctxMenu.x, y: ctxMenu.y }}
          onClose={() => setCtxMenu(null)}
        />
      )}
    </>
  )
}
