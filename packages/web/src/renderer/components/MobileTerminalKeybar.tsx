import { useState, useCallback, useRef } from 'react'
import { ArrowUp, ArrowDown, ArrowLeft, ArrowRight } from 'lucide-react'

interface PillProps {
  children: React.ReactNode
  onPress: (e: React.PointerEvent) => void
  active?: boolean
  className?: string
}

/** Pill button -- rounded-full, compact, iOS 26-native feel */
function Pill({ children, onPress, active, className = '' }: PillProps) {
  return (
    <button
      onPointerDown={onPress}
      className={`shrink-0 flex items-center justify-center rounded-full
                 select-none transition-all h-[32px] px-3
                 ${
                   active
                     ? 'text-white bg-white/[0.18]'
                     : 'text-gray-300 bg-white/[0.07] active:bg-white/[0.16] active:scale-95'
                 } ${className}`}
      style={active ? { boxShadow: 'var(--glass-shadow-thumb)' } : undefined}
    >
      {children}
    </button>
  )
}

const SYMBOLS = ['|', '/', '~', '-', '_', '\\', '"', "'", '.', ':']

interface Props {
  terminalId: string
}

/**
 * iOS 26-native keyboard accessory bar for mobile terminals.
 * Two rows: modifiers + arrows on top, symbols on bottom.
 * Pill-shaped keys grouped by function. Glass material.
 * Uses onPointerDown + preventDefault to avoid stealing focus from xterm.js.
 */
export function MobileTerminalKeybar({ terminalId }: Props) {
  const [ctrlActive, setCtrlActive] = useState(false)
  const ctrlTimeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined)

  const sendKey = useCallback(
    (data: string) => {
      if (ctrlActive && data.length === 1) {
        const code = data.toUpperCase().charCodeAt(0) - 64
        if (code >= 1 && code <= 26) {
          window.api.writeTerminal(terminalId, String.fromCharCode(code))
        } else {
          window.api.writeTerminal(terminalId, data)
        }
        setCtrlActive(false)
        if (ctrlTimeoutRef.current) clearTimeout(ctrlTimeoutRef.current)
      } else {
        window.api.writeTerminal(terminalId, data)
      }
    },
    [terminalId, ctrlActive]
  )

  const handleCtrlToggle = useCallback(() => {
    setCtrlActive((prev) => {
      if (prev) {
        if (ctrlTimeoutRef.current) clearTimeout(ctrlTimeoutRef.current)
        return false
      }
      if (ctrlTimeoutRef.current) clearTimeout(ctrlTimeoutRef.current)
      ctrlTimeoutRef.current = setTimeout(() => setCtrlActive(false), 5000)
      return true
    })
  }, [])

  const press = useCallback(
    (e: React.PointerEvent, data: string) => {
      e.preventDefault()
      e.stopPropagation()
      sendKey(data)
    },
    [sendKey]
  )

  const pressCtrl = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault()
      e.stopPropagation()
      handleCtrlToggle()
    },
    [handleCtrlToggle]
  )

  return (
    <div
      className="flex flex-col gap-1.5 px-3 py-2"
      style={{
        background: 'var(--glass-bg)',
        backdropFilter: 'var(--glass-blur)',
        WebkitBackdropFilter: 'var(--glass-blur)',
        boxShadow: 'var(--glass-shadow)'
      }}
    >
      {/* Row 1: Modifiers + Arrows */}
      <div className="flex items-center gap-1.5">
        {/* Modifier group */}
        <Pill onPress={(e) => press(e, '\x1b')}>
          <span className="text-[11px] font-semibold tracking-wide">ESC</span>
        </Pill>
        <Pill onPress={(e) => press(e, '\x09')}>
          <span className="text-[11px] font-semibold tracking-wide">TAB</span>
        </Pill>
        <Pill onPress={pressCtrl} active={ctrlActive}>
          <span className="text-[11px] font-semibold tracking-wide">CTL</span>
        </Pill>

        {/* Spacer */}
        <div className="flex-1" />

        {/* Arrow group -- d-pad cluster */}
        <div className="flex items-center gap-1">
          <Pill onPress={(e) => press(e, '\x1b[D')} className="!px-2.5">
            <ArrowLeft size={14} strokeWidth={2.5} />
          </Pill>
          <div className="flex flex-col gap-0.5">
            <Pill onPress={(e) => press(e, '\x1b[A')} className="!h-[15px] !px-2">
              <ArrowUp size={11} strokeWidth={2.5} />
            </Pill>
            <Pill onPress={(e) => press(e, '\x1b[B')} className="!h-[15px] !px-2">
              <ArrowDown size={11} strokeWidth={2.5} />
            </Pill>
          </div>
          <Pill onPress={(e) => press(e, '\x1b[C')} className="!px-2.5">
            <ArrowRight size={14} strokeWidth={2.5} />
          </Pill>
        </div>
      </div>

      {/* Row 2: Symbol pills */}
      <div className="flex items-center gap-1.5">
        {SYMBOLS.map((ch) => (
          <Pill key={ch} onPress={(e) => press(e, ch)} className="flex-1 !px-0">
            <span className="text-[13px] font-mono">{ch}</span>
          </Pill>
        ))}
      </div>
    </div>
  )
}
