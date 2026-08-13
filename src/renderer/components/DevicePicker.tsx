import { useState, useEffect, useRef, useLayoutEffect, RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Loader2, Smartphone } from 'lucide-react'
import { AnchorRect, calculatePopoverPosition } from '../lib/popover-position'
import { isSelectable } from '../lib/device-affordance'
import type { DeviceInfo } from '../../shared/types'

interface DevicePickerProps {
  sessionId: string
  onSelect: (device: { udid: string; name: string }) => void
  onClose: () => void
  minWidth?: number
  anchorRef: RefObject<HTMLElement | null>
}

function readAnchorRect(el: HTMLElement): AnchorRect {
  const r = el.getBoundingClientRect()
  return {
    top: r.top,
    left: r.left,
    right: r.right,
    bottom: r.bottom,
    width: r.width,
    height: r.height
  }
}

export function DevicePicker({
  sessionId,
  onSelect,
  onClose,
  minWidth = 240,
  anchorRef
}: DevicePickerProps): React.ReactElement {
  const [devices, setDevices] = useState<DeviceInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null)

  useLayoutEffect(() => {
    const updatePosition = (): void => {
      const anchor = anchorRef.current
      if (!anchor) return
      const popoverRect = ref.current?.getBoundingClientRect()
      const next = calculatePopoverPosition(
        readAnchorRect(anchor),
        { width: popoverRect?.width ?? minWidth, height: popoverRect?.height ?? 200 },
        { width: window.innerWidth, height: window.innerHeight }
      )
      setPosition((prev) =>
        prev && prev.top === next.top && prev.left === next.left
          ? prev
          : { top: next.top, left: next.left }
      )
    }
    updatePosition()
    window.addEventListener('resize', updatePosition)
    window.addEventListener('scroll', updatePosition, true)
    return () => {
      window.removeEventListener('resize', updatePosition)
      window.removeEventListener('scroll', updatePosition, true)
    }
  }, [anchorRef, minWidth])

  useEffect(() => {
    let cancelled = false
    window.api
      .deviceList()
      .then((result) => {
        if (cancelled) return
        setDevices(result)
      })
      .catch((e: unknown) => {
        if (cancelled) return
        // The message carries the fix — no Xcode, no runtime, companion missing.
        // Flattening it to "failed to load" costs the person the one sentence
        // that tells them what to install.
        setError(e instanceof Error ? e.message : String(e))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent): void => {
      const target = e.target as Node
      if (anchorRef.current?.contains(target)) return
      if (ref.current && !ref.current.contains(target)) onClose()
    }
    const handleKeyDown = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('mousedown', handleMouseDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handleMouseDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [onClose, anchorRef])

  return createPortal(
    <div
      ref={ref}
      role="listbox"
      aria-label="Choose a device"
      className="fixed border border-white/[0.08] rounded-lg shadow-xl z-[150] max-h-[280px] overflow-hidden flex flex-col"
      style={{
        background: '#1e1e22',
        minWidth,
        top: position?.top ?? 0,
        left: position?.left ?? 0,
        visibility: position ? 'visible' : 'hidden'
      }}
    >
      <div className="px-3 py-2 border-b border-white/[0.06] text-[10px] uppercase tracking-wide text-gray-500">
        Simulators
      </div>
      <div className="py-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center py-3">
            <Loader2 size={14} className="animate-spin text-gray-500" />
          </div>
        ) : error ? (
          <div className="text-red-400/80 text-[11px] px-3 py-2 whitespace-pre-wrap">{error}</div>
        ) : devices.length === 0 ? (
          <div className="text-gray-500 text-[12px] px-3 py-2">No simulators found</div>
        ) : (
          devices.map((d) => {
            const selectable = isSelectable(d, sessionId)
            return (
              <button
                key={d.udid}
                type="button"
                role="option"
                aria-selected={false}
                disabled={!selectable}
                title={selectable ? d.runtime : `In use by ${d.claimedBy}`}
                onClick={() => selectable && onSelect({ udid: d.udid, name: d.name })}
                className={`w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 transition-colors ${
                  selectable
                    ? 'text-gray-300 hover:bg-white/[0.06]'
                    : 'text-gray-600 cursor-not-allowed'
                }`}
              >
                <Smartphone
                  size={11}
                  className={d.booted ? 'text-green-400/70 shrink-0' : 'text-gray-600 shrink-0'}
                />
                <span className="truncate">{d.name}</span>
                {!selectable && (
                  <span className="text-[9px] text-amber-400/60 ml-auto shrink-0">in use</span>
                )}
                {selectable && d.booted && (
                  <span className="text-[9px] text-green-400/60 ml-auto shrink-0">booted</span>
                )}
              </button>
            )
          })
        )}
      </div>
    </div>,
    document.body
  )
}
