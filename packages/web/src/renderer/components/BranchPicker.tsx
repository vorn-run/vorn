import {
  useState,
  useMemo,
  useRef,
  useEffect,
  useLayoutEffect,
  useCallback,
  RefObject
} from 'react'
import { createPortal } from 'react-dom'
import { GitBranch, Loader2, RefreshCw } from 'lucide-react'
import { AnchorRect, calculatePopoverPosition } from '../lib/popover-position'

interface BranchPickerProps {
  projectPath: string
  currentBranch: string | null
  selectedBranch?: string
  onSelect: (branch: string) => void
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

export function BranchPicker({
  projectPath,
  currentBranch,
  selectedBranch,
  onSelect,
  onClose,
  minWidth = 220,
  anchorRef
}: BranchPickerProps) {
  const [localBranches, setLocalBranches] = useState<string[]>([])
  const [remoteBranches, setRemoteBranches] = useState<string[]>([])
  const [filter, setFilter] = useState('')
  const [loadingLocal, setLoadingLocal] = useState(true)
  const [loadingRemotes, setLoadingRemotes] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null)

  useLayoutEffect(() => {
    const updatePosition = () => {
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

    let frame = 0
    const scheduleUpdate = () => {
      if (frame) return
      frame = window.requestAnimationFrame(() => {
        frame = 0
        updatePosition()
      })
    }
    window.addEventListener('resize', scheduleUpdate)
    window.addEventListener('scroll', scheduleUpdate, true)

    const popover = ref.current
    const resizeObserver =
      popover && typeof ResizeObserver !== 'undefined' ? new ResizeObserver(scheduleUpdate) : null
    if (popover && resizeObserver) resizeObserver.observe(popover)

    return () => {
      if (frame) window.cancelAnimationFrame(frame)
      window.removeEventListener('resize', scheduleUpdate)
      window.removeEventListener('scroll', scheduleUpdate, true)
      resizeObserver?.disconnect()
    }
  }, [anchorRef, minWidth])

  useEffect(() => {
    setLoadingLocal(true)
    setRemoteBranches([])
    setFilter('')
    window.api
      .listBranches(projectPath)
      .then((result) => setLocalBranches(result.local))
      .catch(() => setLocalBranches([]))
      .finally(() => setLoadingLocal(false))
  }, [projectPath])

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (anchorRef.current?.contains(target)) return
      if (ref.current && !ref.current.contains(target)) onClose()
    }
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('mousedown', handleMouseDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handleMouseDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [onClose, anchorRef])

  useEffect(() => {
    setTimeout(() => inputRef.current?.focus(), 0)
  }, [])

  const handleFetchRemotes = useCallback(async () => {
    if (loadingRemotes) return
    setLoadingRemotes(true)
    try {
      const remotes = await window.api.listRemoteBranches(projectPath)
      setRemoteBranches(remotes.filter((r) => !localBranches.includes(r)))
    } catch {
      setRemoteBranches([])
    } finally {
      setLoadingRemotes(false)
    }
  }, [projectPath, loadingRemotes, localBranches])

  const allBranches = useMemo(
    () => [
      ...localBranches.map((b) => ({ name: b, isRemote: false })),
      ...remoteBranches.map((b) => ({ name: b, isRemote: true }))
    ],
    [localBranches, remoteBranches]
  )

  const filtered = useMemo(
    () =>
      filter
        ? allBranches.filter((b) => b.name.toLowerCase().includes(filter.toLowerCase()))
        : allBranches,
    [allBranches, filter]
  )

  const active = selectedBranch ?? currentBranch

  return createPortal(
    <div
      ref={ref}
      className="fixed border border-white/[0.08] rounded-lg shadow-xl z-[150] max-h-[280px] overflow-hidden flex flex-col"
      style={{
        background: 'var(--color-surface-overlay)',
        minWidth,
        top: position?.top ?? 0,
        left: position?.left ?? 0,
        visibility: position ? 'visible' : 'hidden'
      }}
    >
      <div className="p-2 border-b border-white/[0.06] flex items-center gap-1">
        <input
          ref={inputRef}
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter branches..."
          className="flex-1 px-2 py-1 bg-transparent text-xs text-gray-200 placeholder-gray-600 focus:outline-none"
        />
        <button
          onClick={handleFetchRemotes}
          disabled={loadingRemotes}
          className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
          title="Fetch remotes"
        >
          {loadingRemotes ? (
            <Loader2 size={10} className="animate-spin" />
          ) : (
            <RefreshCw size={10} />
          )}
        </button>
      </div>
      <div className="py-1 overflow-y-auto">
        {loadingLocal ? (
          <div className="flex items-center justify-center py-3">
            <Loader2 size={14} className="animate-spin text-gray-500" />
          </div>
        ) : filtered.length === 0 ? (
          <div className="text-gray-500 text-[12px] px-3 py-2">No branches found</div>
        ) : (
          filtered.map((b) => (
            <button
              key={`${b.name}-${b.isRemote}`}
              onClick={() => onSelect(b.name)}
              className={`w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 hover:bg-white/[0.06] transition-colors ${
                active === b.name ? 'text-white bg-white/[0.04]' : 'text-gray-400'
              }`}
            >
              <GitBranch size={10} className={b.isRemote ? 'text-blue-400' : 'text-gray-500'} />
              <span className="truncate">{b.name}</span>
              {b.isRemote && <span className="text-[9px] text-blue-400/60 ml-auto">remote</span>}
              {b.name === currentBranch && (
                <span className="text-[9px] text-green-400/60 ml-auto">current</span>
              )}
            </button>
          ))
        )}
      </div>
    </div>,
    document.body
  )
}
