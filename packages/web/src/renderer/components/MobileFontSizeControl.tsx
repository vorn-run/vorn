import { useState, useCallback, useEffect } from 'react'
import { Minus, Plus, Type } from 'lucide-react'
import { setAllTerminalsFontSize, getCurrentTerminalFontSize } from '../lib/terminal-registry'
import { useAppStore } from '../stores'

const MIN_FONT_SIZE = 8
const MAX_FONT_SIZE = 28

/**
 * Floating font size control for mobile devices.
 * Shows a compact A+/A- pill that expands on tap to reveal size controls.
 * Only renders on touch devices.
 */
export function MobileFontSizeControl() {
  const [isExpanded, setIsExpanded] = useState(false)
  const [fontSize, setFontSize] = useState(() => getCurrentTerminalFontSize())

  // Auto-collapse after 4s of inactivity
  useEffect(() => {
    if (!isExpanded) return
    const timer = setTimeout(() => setIsExpanded(false), 4000)
    return () => clearTimeout(timer)
  }, [isExpanded, fontSize])

  const updateSize = useCallback((delta: number) => {
    const current = getCurrentTerminalFontSize()
    const newSize = Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, current + delta))
    setAllTerminalsFontSize(newSize)
    setFontSize(newSize)

    // Persist to config
    const state = useAppStore.getState()
    if (state.config) {
      const updated = {
        ...state.config,
        defaults: { ...state.config.defaults, fontSize: newSize }
      }
      window.api.saveConfig(updated)
      state.setConfig(updated)
    }
  }, [])

  if (!isExpanded) {
    return (
      <button
        onClick={() => {
          setFontSize(getCurrentTerminalFontSize())
          setIsExpanded(true)
        }}
        className="flex items-center justify-center w-8 h-8 rounded-lg
                   text-gray-400 hover:text-white transition-colors"
        style={{
          background: 'var(--glass-bg, rgba(255,255,255,0.1))',
          backdropFilter: 'var(--glass-blur, none)',
          WebkitBackdropFilter: 'var(--glass-blur, none)',
          boxShadow: 'var(--glass-shadow, none)'
        }}
        title="Font size"
      >
        <Type size={14} strokeWidth={2} />
      </button>
    )
  }

  return (
    <div
      className="flex items-center gap-1 rounded-lg px-1 py-0.5"
      style={{
        background: 'var(--glass-bg)',
        backdropFilter: 'var(--glass-blur)',
        WebkitBackdropFilter: 'var(--glass-blur)',
        boxShadow: 'var(--glass-shadow)'
      }}
    >
      <button
        onClick={() => updateSize(-1)}
        disabled={fontSize <= MIN_FONT_SIZE}
        className="w-7 h-7 flex items-center justify-center rounded-md
                   text-gray-300 hover:text-white active:bg-white/[0.15]
                   transition-colors disabled:opacity-30 disabled:pointer-events-none"
      >
        <Minus size={14} strokeWidth={2.5} />
      </button>
      <span className="text-[11px] font-mono text-gray-300 w-6 text-center tabular-nums select-none">
        {fontSize}
      </span>
      <button
        onClick={() => updateSize(1)}
        disabled={fontSize >= MAX_FONT_SIZE}
        className="w-7 h-7 flex items-center justify-center rounded-md
                   text-gray-300 hover:text-white active:bg-white/[0.15]
                   transition-colors disabled:opacity-30 disabled:pointer-events-none"
      >
        <Plus size={14} strokeWidth={2.5} />
      </button>
    </div>
  )
}
